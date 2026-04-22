use std::collections::HashMap;
use std::path::Path;

use inkwell::{
    AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel,
    builder::Builder,
    context::Context,
    module::{Linkage, Module},
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
    types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum},
    values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue},
};

use beer_ast::*;
use beer_errors::CompileError;
use beer_span::Span;

pub fn compile(program: &Program, obj_path: Option<&Path>) -> Result<(), CompileError> {
    let ctx = Context::create();
    let mut cg = CodeGen::new(&ctx);
    cg.compile_program(program)?;

    Target::initialize_all(&InitializationConfig::default());
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| CompileError::new(e.to_string()))?;
    let tm = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| CompileError::new("failed to create target machine"))?;

    cg.module.set_triple(&triple);
    cg.module
        .set_data_layout(&tm.get_target_data().get_data_layout());

    cg.module
        .verify()
        .map_err(|e| CompileError::new(format!("LLVM module verification failed:\n{}", e)))?;

    if let Some(p) = obj_path {
        tm.write_to_file(&cg.module, FileType::Object, p)
            .map_err(|e| CompileError::new(e.to_string()))?;
    }
    Ok(())
}

fn be(e: impl std::fmt::Display) -> CompileError {
    CompileError::new(e.to_string())
}

struct CodeGen<'ctx> {
    ctx: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    funcs: HashMap<String, FunctionValue<'ctx>>,
    fn_sigs: HashMap<String, (Vec<Type>, Type)>,
    vars: HashMap<String, (PointerValue<'ctx>, Type)>,

    current_fn: Option<FunctionValue<'ctx>>,
    current_ret: Type,
    current_is_main_void: bool,

    printf: Option<FunctionValue<'ctx>>,
    fmt_int: Option<PointerValue<'ctx>>,
    fmt_float: Option<PointerValue<'ctx>>,
    fmt_str: Option<PointerValue<'ctx>>,
    str_true: Option<PointerValue<'ctx>>,
    str_false: Option<PointerValue<'ctx>>,
}

impl<'ctx> CodeGen<'ctx> {
    fn new(ctx: &'ctx Context) -> Self {
        let module = ctx.create_module("beer");
        let builder = ctx.create_builder();
        Self {
            ctx,
            module,
            builder,
            funcs: HashMap::new(),
            fn_sigs: HashMap::new(),
            vars: HashMap::new(),
            current_fn: None,
            current_ret: Type::Void,
            current_is_main_void: false,
            printf: None,
            fmt_int: None,
            fmt_float: None,
            fmt_str: None,
            str_true: None,
            str_false: None,
        }
    }

    fn compile_program(&mut self, prog: &Program) -> Result<(), CompileError> {
        for f in &prog.funcs {
            self.declare_function(f)?;
        }
        for f in &prog.funcs {
            self.define_function(f)?;
        }
        Ok(())
    }

    fn declare_function(&mut self, f: &Func) -> Result<(), CompileError> {
        if self.funcs.contains_key(&f.name) {
            return Err(CompileError::at(
                f.span,
                format!("duplicate function: {}", f.name),
            ));
        }
        let param_tys: Vec<BasicMetadataTypeEnum> = f
            .params
            .iter()
            .map(|(_, t)| self.basic_type(*t).into())
            .collect();

        let is_main_void = f.name == "main" && f.ret == Type::Void;
        let fn_ty = if is_main_void {
            self.ctx.i32_type().fn_type(&param_tys, false)
        } else {
            match f.ret {
                Type::Void => self.ctx.void_type().fn_type(&param_tys, false),
                t => self.basic_type(t).fn_type(&param_tys, false),
            }
        };

        let fv = self.module.add_function(&f.name, fn_ty, None);
        for (i, (name, _)) in f.params.iter().enumerate() {
            if let Some(p) = fv.get_nth_param(i as u32) {
                p.set_name(name);
            }
        }
        self.funcs.insert(f.name.clone(), fv);
        self.fn_sigs.insert(
            f.name.clone(),
            (f.params.iter().map(|(_, t)| *t).collect(), f.ret),
        );
        Ok(())
    }

    fn define_function(&mut self, f: &Func) -> Result<(), CompileError> {
        let fv = *self.funcs.get(&f.name).unwrap();
        let entry = self.ctx.append_basic_block(fv, "entry");
        self.builder.position_at_end(entry);

        self.vars.clear();
        self.current_fn = Some(fv);
        self.current_ret = f.ret;
        self.current_is_main_void = f.name == "main" && f.ret == Type::Void;

        for (i, (pname, pty)) in f.params.iter().enumerate() {
            let param = fv.get_nth_param(i as u32).unwrap();
            let slot = self
                .builder
                .build_alloca(self.basic_type(*pty), pname)
                .map_err(be)?;
            self.builder.build_store(slot, param).map_err(be)?;
            self.vars.insert(pname.clone(), (slot, *pty));
        }

        let terminated = self.lower_block(&f.body)?;
        if !terminated {
            self.emit_implicit_return()?;
        }

        Ok(())
    }

    fn emit_implicit_return(&mut self) -> Result<(), CompileError> {
        if self.current_is_main_void {
            let zero = self.ctx.i32_type().const_int(0, false);
            self.builder.build_return(Some(&zero)).map_err(be)?;
        } else {
            match self.current_ret {
                Type::Void => {
                    self.builder.build_return(None).map_err(be)?;
                }
                _ => {
                    self.builder.build_unreachable().map_err(be)?;
                }
            }
        }
        Ok(())
    }

    fn lower_block(&mut self, stmts: &[Stmt]) -> Result<bool, CompileError> {
        for s in stmts {
            let terminated = self.lower_stmt(s)?;
            if terminated {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn lower_stmt(&mut self, s: &Stmt) -> Result<bool, CompileError> {
        match &s.kind {
            StmtKind::Let { name, value } => {
                let ty = self.expr_type(value)?;
                if ty == Type::Void {
                    return Err(CompileError::at(
                        value.span,
                        format!("cannot bind void expression to `{}`", name),
                    ));
                }
                let v = self.lower_expr(value)?;
                let slot = self
                    .builder
                    .build_alloca(self.basic_type(ty), name)
                    .map_err(be)?;
                self.builder.build_store(slot, v).map_err(be)?;
                self.vars.insert(name.clone(), (slot, ty));
                Ok(false)
            }
            StmtKind::Assign { name, value } => {
                let (slot, ty) = *self.vars.get(name).ok_or_else(|| {
                    CompileError::at(s.span, format!("unknown variable: {}", name))
                })?;
                let vt = self.expr_type(value)?;
                if vt != ty {
                    return Err(CompileError::at(
                        value.span,
                        format!(
                            "type mismatch assigning to `{}`: expected {:?}, got {:?}",
                            name, ty, vt
                        ),
                    ));
                }
                let v = self.lower_expr(value)?;
                self.builder.build_store(slot, v).map_err(be)?;
                Ok(false)
            }
            StmtKind::If { cond, then_block, else_block } => {
                let cond_v = self.lower_expr_as_bool(cond)?;
                let fv = self.current_fn.unwrap();
                let then_bb = self.ctx.append_basic_block(fv, "then");
                let else_bb = self.ctx.append_basic_block(fv, "else");
                let merge_bb = self.ctx.append_basic_block(fv, "merge");

                self.builder
                    .build_conditional_branch(cond_v, then_bb, else_bb)
                    .map_err(be)?;

                self.builder.position_at_end(then_bb);
                let then_term = self.lower_block(then_block)?;
                if !then_term {
                    self.builder.build_unconditional_branch(merge_bb).map_err(be)?;
                }

                self.builder.position_at_end(else_bb);
                let else_term = if let Some(eb) = else_block {
                    self.lower_block(eb)?
                } else {
                    false
                };
                if !else_term {
                    self.builder.build_unconditional_branch(merge_bb).map_err(be)?;
                }

                if then_term && else_term && else_block.is_some() {
                    self.builder.position_at_end(merge_bb);
                    self.builder.build_unreachable().map_err(be)?;
                    Ok(true)
                } else {
                    self.builder.position_at_end(merge_bb);
                    Ok(false)
                }
            }
            StmtKind::While { cond, body } => {
                let fv = self.current_fn.unwrap();
                let cond_bb = self.ctx.append_basic_block(fv, "while_cond");
                let body_bb = self.ctx.append_basic_block(fv, "while_body");
                let after_bb = self.ctx.append_basic_block(fv, "while_after");

                self.builder.build_unconditional_branch(cond_bb).map_err(be)?;

                self.builder.position_at_end(cond_bb);
                let cv = self.lower_expr_as_bool(cond)?;
                self.builder
                    .build_conditional_branch(cv, body_bb, after_bb)
                    .map_err(be)?;

                self.builder.position_at_end(body_bb);
                let body_term = self.lower_block(body)?;
                if !body_term {
                    self.builder.build_unconditional_branch(cond_bb).map_err(be)?;
                }

                self.builder.position_at_end(after_bb);
                Ok(false)
            }
            StmtKind::Return(None) => {
                if self.current_is_main_void {
                    let zero = self.ctx.i32_type().const_int(0, false);
                    self.builder.build_return(Some(&zero)).map_err(be)?;
                } else if self.current_ret == Type::Void {
                    self.builder.build_return(None).map_err(be)?;
                } else {
                    return Err(CompileError::at(s.span, "missing return value"));
                }
                Ok(true)
            }
            StmtKind::Return(Some(e)) => {
                if self.current_ret == Type::Void {
                    return Err(CompileError::at(
                        e.span,
                        "unexpected return value in void function",
                    ));
                }
                let et = self.expr_type(e)?;
                if et != self.current_ret {
                    return Err(CompileError::at(
                        e.span,
                        format!(
                            "return type mismatch: expected {:?}, got {:?}",
                            self.current_ret, et
                        ),
                    ));
                }
                let v = self.lower_expr(e)?;
                self.builder.build_return(Some(&v)).map_err(be)?;
                Ok(true)
            }
            StmtKind::Expr(e) => {
                self.lower_expr(e)?;
                Ok(false)
            }
        }
    }

    fn lower_expr_as_bool(&mut self, e: &Expr) -> Result<IntValue<'ctx>, CompileError> {
        if self.expr_type(e)? != Type::Bool {
            return Err(CompileError::at(e.span, "expected boolean condition"));
        }
        let v = self.lower_expr(e)?;
        Ok(v.into_int_value())
    }

    fn lower_expr(&mut self, e: &Expr) -> Result<BasicValueEnum<'ctx>, CompileError> {
        match &e.kind {
            ExprKind::Int(n) => Ok(self.ctx.i64_type().const_int(*n as u64, true).into()),
            ExprKind::Float(f) => Ok(self.ctx.f64_type().const_float(*f).into()),
            ExprKind::Bool(b) => Ok(self
                .ctx
                .bool_type()
                .const_int(if *b { 1 } else { 0 }, false)
                .into()),
            ExprKind::Str(s) => {
                let g = self
                    .builder
                    .build_global_string_ptr(s, "str")
                    .map_err(be)?;
                Ok(g.as_pointer_value().into())
            }
            ExprKind::Ident(name) => {
                let (slot, ty) = *self.vars.get(name).ok_or_else(|| {
                    CompileError::at(e.span, format!("unknown variable: {}", name))
                })?;
                let v = self
                    .builder
                    .build_load(self.basic_type(ty), slot, name)
                    .map_err(be)?;
                Ok(v)
            }
            ExprKind::Unary { op, expr } => {
                let v = self.lower_expr(expr)?;
                match op {
                    UnOp::Neg => match v {
                        BasicValueEnum::IntValue(iv) => Ok(self
                            .builder
                            .build_int_neg(iv, "neg")
                            .map_err(be)?
                            .into()),
                        BasicValueEnum::FloatValue(fv) => Ok(self
                            .builder
                            .build_float_neg(fv, "fneg")
                            .map_err(be)?
                            .into()),
                        _ => unreachable!("type check should have rejected"),
                    },
                    UnOp::Not => {
                        let iv = v.into_int_value();
                        let r = self.builder.build_not(iv, "not").map_err(be)?;
                        Ok(r.into())
                    }
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    return self.lower_short_circuit(*op, lhs, rhs);
                }
                let l = self.lower_expr(lhs)?;
                let r = self.lower_expr(rhs)?;
                self.lower_binop(*op, l, r, e.span)
            }
            ExprKind::Call { name, args } => self.lower_call(name, args, e.span),
            ExprKind::Cast { expr, target } => {
                let src_ty = self.expr_type(expr)?;
                let v = self.lower_expr(expr)?;
                self.lower_cast(v, src_ty, *target, e.span)
            }
        }
    }

    fn lower_cast(
        &mut self,
        v: BasicValueEnum<'ctx>,
        from: Type,
        to: Type,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        match (from, to) {
            (a, b) if a == b => Ok(v),
            (Type::Int, Type::Float) => {
                let iv = v.into_int_value();
                Ok(self
                    .builder
                    .build_signed_int_to_float(iv, self.ctx.f64_type(), "to_f")
                    .map_err(be)?
                    .into())
            }
            (Type::Float, Type::Int) => {
                let fv = v.into_float_value();
                Ok(self
                    .builder
                    .build_float_to_signed_int(fv, self.ctx.i64_type(), "to_i")
                    .map_err(be)?
                    .into())
            }
            _ => Err(CompileError::at(
                span,
                format!("cannot cast {:?} to {:?}", from, to),
            )),
        }
    }

    fn lower_binop(
        &mut self,
        op: BinOp,
        l: BasicValueEnum<'ctx>,
        r: BasicValueEnum<'ctx>,
        _span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        let b = &self.builder;
        match (l, r) {
            (BasicValueEnum::IntValue(li), BasicValueEnum::IntValue(ri)) => {
                let v: IntValue = match op {
                    BinOp::Add => b.build_int_add(li, ri, "add").map_err(be)?,
                    BinOp::Sub => b.build_int_sub(li, ri, "sub").map_err(be)?,
                    BinOp::Mul => b.build_int_mul(li, ri, "mul").map_err(be)?,
                    BinOp::Div => b.build_int_signed_div(li, ri, "div").map_err(be)?,
                    BinOp::Eq => b
                        .build_int_compare(IntPredicate::EQ, li, ri, "eq")
                        .map_err(be)?,
                    BinOp::Ne => b
                        .build_int_compare(IntPredicate::NE, li, ri, "ne")
                        .map_err(be)?,
                    BinOp::Lt => b
                        .build_int_compare(IntPredicate::SLT, li, ri, "lt")
                        .map_err(be)?,
                    BinOp::Le => b
                        .build_int_compare(IntPredicate::SLE, li, ri, "le")
                        .map_err(be)?,
                    BinOp::Gt => b
                        .build_int_compare(IntPredicate::SGT, li, ri, "gt")
                        .map_err(be)?,
                    BinOp::Ge => b
                        .build_int_compare(IntPredicate::SGE, li, ri, "ge")
                        .map_err(be)?,
                    BinOp::And | BinOp::Or => unreachable!("logical ops go through lower_short_circuit"),
                };
                Ok(v.into())
            }
            (BasicValueEnum::FloatValue(lf), BasicValueEnum::FloatValue(rf)) => {
                let v: BasicValueEnum = match op {
                    BinOp::Add => b.build_float_add(lf, rf, "fadd").map_err(be)?.into(),
                    BinOp::Sub => b.build_float_sub(lf, rf, "fsub").map_err(be)?.into(),
                    BinOp::Mul => b.build_float_mul(lf, rf, "fmul").map_err(be)?.into(),
                    BinOp::Div => b.build_float_div(lf, rf, "fdiv").map_err(be)?.into(),
                    BinOp::Eq => b
                        .build_float_compare(FloatPredicate::OEQ, lf, rf, "feq")
                        .map_err(be)?
                        .into(),
                    BinOp::Ne => b
                        .build_float_compare(FloatPredicate::ONE, lf, rf, "fne")
                        .map_err(be)?
                        .into(),
                    BinOp::Lt => b
                        .build_float_compare(FloatPredicate::OLT, lf, rf, "flt")
                        .map_err(be)?
                        .into(),
                    BinOp::Le => b
                        .build_float_compare(FloatPredicate::OLE, lf, rf, "fle")
                        .map_err(be)?
                        .into(),
                    BinOp::Gt => b
                        .build_float_compare(FloatPredicate::OGT, lf, rf, "fgt")
                        .map_err(be)?
                        .into(),
                    BinOp::Ge => b
                        .build_float_compare(FloatPredicate::OGE, lf, rf, "fge")
                        .map_err(be)?
                        .into(),
                    BinOp::And | BinOp::Or => unreachable!("logical op on float"),
                };
                Ok(v)
            }
            _ => unreachable!("mixed operand types reached lower_binop"),
        }
    }

    fn lower_short_circuit(
        &mut self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        let fv = self.current_fn.unwrap();
        let rhs_bb = self.ctx.append_basic_block(fv, "sc_rhs");
        let short_bb = self.ctx.append_basic_block(fv, "sc_short");
        let merge_bb = self.ctx.append_basic_block(fv, "sc_merge");

        let lhs_val = self.lower_expr(lhs)?.into_int_value();
        match op {
            BinOp::And => {
                self.builder
                    .build_conditional_branch(lhs_val, rhs_bb, short_bb)
                    .map_err(be)?;
            }
            BinOp::Or => {
                self.builder
                    .build_conditional_branch(lhs_val, short_bb, rhs_bb)
                    .map_err(be)?;
            }
            _ => unreachable!("lower_short_circuit called with non-logical op"),
        }

        self.builder.position_at_end(rhs_bb);
        let rhs_val = self.lower_expr(rhs)?.into_int_value();
        let rhs_end_bb = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(merge_bb).map_err(be)?;

        self.builder.position_at_end(short_bb);
        self.builder.build_unconditional_branch(merge_bb).map_err(be)?;

        self.builder.position_at_end(merge_bb);
        let short_val = self
            .ctx
            .bool_type()
            .const_int(if matches!(op, BinOp::Or) { 1 } else { 0 }, false);
        let phi = self.builder.build_phi(self.ctx.bool_type(), "sc").map_err(be)?;
        phi.add_incoming(&[(&rhs_val, rhs_end_bb), (&short_val, short_bb)]);
        Ok(phi.as_basic_value())
    }

    fn lower_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        if name == "print" {
            return self.lower_print(args, span);
        }

        let fv = *self
            .funcs
            .get(name)
            .ok_or_else(|| CompileError::at(span, format!("unknown function: {}", name)))?;
        let (param_tys, ret_ty) = self.fn_sigs.get(name).cloned().unwrap();

        if args.len() != param_tys.len() {
            return Err(CompileError::at(
                span,
                format!("`{}` expects {} args, got {}", name, param_tys.len(), args.len()),
            ));
        }
        let mut lowered: Vec<BasicMetadataValueEnum> = Vec::with_capacity(args.len());
        for (a, pty) in args.iter().zip(&param_tys) {
            let at = self.expr_type(a)?;
            if at != *pty {
                return Err(CompileError::at(
                    a.span,
                    format!(
                        "arg type mismatch calling `{}`: expected {:?}, got {:?}",
                        name, pty, at
                    ),
                ));
            }
            lowered.push(self.lower_expr(a)?.into());
        }

        let call = self.builder.build_call(fv, &lowered, "call").map_err(be)?;

        if ret_ty == Type::Void {
            Ok(self.ctx.i64_type().const_zero().into())
        } else {
            Ok(call.try_as_basic_value().basic().ok_or_else(|| {
                CompileError::at(span, "expected value from function call")
            })?)
        }
    }

    fn lower_print(
        &mut self,
        args: &[Expr],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CompileError> {
        if args.len() != 1 {
            return Err(CompileError::at(
                span,
                format!("print expects 1 arg, got {}", args.len()),
            ));
        }
        let ty = self.expr_type(&args[0])?;
        let printf = self.ensure_printf();

        match ty {
            Type::Int => {
                let fmt = self.ensure_fmt_int()?;
                let v = self.lower_expr(&args[0])?;
                self.builder
                    .build_call(printf, &[fmt.into(), v.into()], "printf")
                    .map_err(be)?;
            }
            Type::Float => {
                let fmt = self.ensure_fmt_float()?;
                let v = self.lower_expr(&args[0])?;
                self.builder
                    .build_call(printf, &[fmt.into(), v.into()], "printf")
                    .map_err(be)?;
            }
            Type::Str => {
                let fmt = self.ensure_fmt_str()?;
                let v = self.lower_expr(&args[0])?;
                self.builder
                    .build_call(printf, &[fmt.into(), v.into()], "printf")
                    .map_err(be)?;
            }
            Type::Bool => {
                let fmt = self.ensure_fmt_str()?;
                let t = self.ensure_str_true()?;
                let f = self.ensure_str_false()?;
                let cond = self.lower_expr(&args[0])?.into_int_value();
                let selected = self.builder.build_select(cond, t, f, "sel").map_err(be)?;
                self.builder
                    .build_call(printf, &[fmt.into(), selected.into()], "printf")
                    .map_err(be)?;
            }
            Type::Void => return Err(CompileError::at(args[0].span, "cannot print void")),
        }
        Ok(self.ctx.i64_type().const_zero().into())
    }

    fn ensure_printf(&mut self) -> FunctionValue<'ctx> {
        if let Some(f) = self.printf {
            return f;
        }
        let pty = self.ctx.ptr_type(AddressSpace::default());
        let ty = self.ctx.i32_type().fn_type(&[pty.into()], true);
        let f = self
            .module
            .add_function("printf", ty, Some(Linkage::External));
        self.printf = Some(f);
        f
    }

    fn ensure_fmt_int(&mut self) -> Result<PointerValue<'ctx>, CompileError> {
        if let Some(p) = self.fmt_int {
            return Ok(p);
        }
        let g = self
            .builder
            .build_global_string_ptr("%lld\n", "fmt_int")
            .map_err(be)?;
        let p = g.as_pointer_value();
        self.fmt_int = Some(p);
        Ok(p)
    }

    fn ensure_fmt_float(&mut self) -> Result<PointerValue<'ctx>, CompileError> {
        if let Some(p) = self.fmt_float {
            return Ok(p);
        }
        let g = self
            .builder
            .build_global_string_ptr("%g\n", "fmt_float")
            .map_err(be)?;
        let p = g.as_pointer_value();
        self.fmt_float = Some(p);
        Ok(p)
    }

    fn ensure_fmt_str(&mut self) -> Result<PointerValue<'ctx>, CompileError> {
        if let Some(p) = self.fmt_str {
            return Ok(p);
        }
        let g = self
            .builder
            .build_global_string_ptr("%s\n", "fmt_str")
            .map_err(be)?;
        let p = g.as_pointer_value();
        self.fmt_str = Some(p);
        Ok(p)
    }

    fn ensure_str_true(&mut self) -> Result<PointerValue<'ctx>, CompileError> {
        if let Some(p) = self.str_true {
            return Ok(p);
        }
        let g = self
            .builder
            .build_global_string_ptr("true", "str_true")
            .map_err(be)?;
        let p = g.as_pointer_value();
        self.str_true = Some(p);
        Ok(p)
    }

    fn ensure_str_false(&mut self) -> Result<PointerValue<'ctx>, CompileError> {
        if let Some(p) = self.str_false {
            return Ok(p);
        }
        let g = self
            .builder
            .build_global_string_ptr("false", "str_false")
            .map_err(be)?;
        let p = g.as_pointer_value();
        self.str_false = Some(p);
        Ok(p)
    }

    fn basic_type(&self, t: Type) -> BasicTypeEnum<'ctx> {
        match t {
            Type::Int => self.ctx.i64_type().into(),
            Type::Float => self.ctx.f64_type().into(),
            Type::Bool => self.ctx.bool_type().into(),
            Type::Str => self.ctx.ptr_type(AddressSpace::default()).into(),
            Type::Void => unreachable!("void is not a basic type"),
        }
    }

    fn expr_type(&self, e: &Expr) -> Result<Type, CompileError> {
        match &e.kind {
            ExprKind::Int(_) => Ok(Type::Int),
            ExprKind::Float(_) => Ok(Type::Float),
            ExprKind::Bool(_) => Ok(Type::Bool),
            ExprKind::Str(_) => Ok(Type::Str),
            ExprKind::Ident(n) => self.vars.get(n).map(|(_, t)| *t).ok_or_else(|| {
                CompileError::at(e.span, format!("unknown variable: {}", n))
            }),
            ExprKind::Binary { op, lhs, rhs } => {
                let lt = self.expr_type(lhs)?;
                let rt = self.expr_type(rhs)?;
                check_binop(*op, lt, rt, e.span)
            }
            ExprKind::Unary { op, expr } => {
                let t = self.expr_type(expr)?;
                check_unop(*op, t, e.span)
            }
            ExprKind::Call { name, .. } => {
                if name == "print" {
                    return Ok(Type::Void);
                }
                self.fn_sigs.get(name).map(|(_, r)| *r).ok_or_else(|| {
                    CompileError::at(e.span, format!("unknown function: {}", name))
                })
            }
            ExprKind::Cast { expr, target } => {
                let src = self.expr_type(expr)?;
                check_cast(src, *target, e.span)
            }
        }
    }
}

fn check_cast(from: Type, to: Type, span: Span) -> Result<Type, CompileError> {
    match (from, to) {
        (a, b) if a == b => Ok(to),
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => Ok(to),
        _ => Err(CompileError::at(
            span,
            format!("cannot cast {:?} to {:?}", from, to),
        )),
    }
}

fn check_binop(op: BinOp, l: Type, r: Type, span: Span) -> Result<Type, CompileError> {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div => match (l, r) {
            (Type::Int, Type::Int) => Ok(Type::Int),
            (Type::Float, Type::Float) => Ok(Type::Float),
            _ => Err(CompileError::at(
                span,
                format!(
                    "operator {:?} requires matching int or float operands, got {:?} and {:?}",
                    op, l, r
                ),
            )),
        },
        Lt | Le | Gt | Ge => match (l, r) {
            (Type::Int, Type::Int) | (Type::Float, Type::Float) => Ok(Type::Bool),
            _ => Err(CompileError::at(
                span,
                format!(
                    "operator {:?} requires matching int or float operands, got {:?} and {:?}",
                    op, l, r
                ),
            )),
        },
        Eq | Ne => {
            if l != r {
                return Err(CompileError::at(
                    span,
                    format!(
                        "operator {:?} requires matching operand types, got {:?} and {:?}",
                        op, l, r
                    ),
                ));
            }
            match l {
                Type::Int | Type::Float | Type::Bool => Ok(Type::Bool),
                Type::Str => Err(CompileError::at(
                    span,
                    "string equality is not supported yet",
                )),
                Type::Void => Err(CompileError::at(span, "cannot compare void values")),
            }
        }
        And | Or => {
            if l == Type::Bool && r == Type::Bool {
                Ok(Type::Bool)
            } else {
                Err(CompileError::at(
                    span,
                    format!("operator {:?} requires bool operands, got {:?} and {:?}", op, l, r),
                ))
            }
        }
    }
}

fn check_unop(op: UnOp, t: Type, span: Span) -> Result<Type, CompileError> {
    match op {
        UnOp::Neg => {
            if t == Type::Int || t == Type::Float {
                Ok(t)
            } else {
                Err(CompileError::at(
                    span,
                    format!("unary `-` requires int or float, got {:?}", t),
                ))
            }
        }
        UnOp::Not => {
            if t == Type::Bool {
                Ok(Type::Bool)
            } else {
                Err(CompileError::at(
                    span,
                    format!("unary `!` requires bool, got {:?}", t),
                ))
            }
        }
    }
}
