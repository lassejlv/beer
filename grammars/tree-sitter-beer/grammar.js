/**
 * Tree-sitter grammar for the `beer` language.
 *
 * Mirrors the handwritten parser at crates/parser, with the same
 * Pratt-style precedence (or < and < comparisons < + - < * / < as < unary).
 */
module.exports = grammar({
  name: 'beer',

  extras: $ => [/\s/, $.line_comment],

  word: $ => $.identifier,

  rules: {
    source_file: $ => seq(
      repeat($.use_declaration),
      repeat($.function_definition),
    ),

    use_declaration: $ => seq('use', field('path', $.string_literal)),

    function_definition: $ => seq(
      'fn',
      field('name', $.identifier),
      field('parameters', $.parameter_list),
      optional(seq('->', field('return_type', $.type))),
      field('body', $.block),
    ),

    parameter_list: $ => seq(
      '(',
      optional(seq($.parameter, repeat(seq(',', $.parameter)))),
      ')',
    ),

    parameter: $ => seq(
      field('name', $.identifier),
      ':',
      field('type', $.type),
    ),

    type: $ => choice('int', 'float', 'bool', 'str'),

    block: $ => seq('{', repeat($.statement), '}'),

    statement: $ => choice(
      $.let_statement,
      $.assignment_statement,
      $.if_statement,
      $.while_statement,
      $.return_statement,
      $.expression_statement,
    ),

    let_statement: $ => seq(
      'let',
      field('name', $.identifier),
      '=',
      field('value', $._expression),
    ),

    assignment_statement: $ => seq(
      field('name', $.identifier),
      '=',
      field('value', $._expression),
    ),

    if_statement: $ => seq(
      'if',
      field('condition', $._expression),
      field('consequence', $.block),
      optional(seq('else', field('alternative', $.block))),
    ),

    while_statement: $ => seq(
      'while',
      field('condition', $._expression),
      field('body', $.block),
    ),

    // prec.right: when the token after `return` could be the start of an
    // expression, greedily consume it as the return value (our language has
    // no statement terminator, so the naive `seq` is ambiguous).
    return_statement: $ => prec.right(seq('return', optional(field('value', $._expression)))),

    expression_statement: $ => $._expression,

    _expression: $ => choice(
      $.binary_expression,
      $.unary_expression,
      $.cast_expression,
      $.call_expression,
      $.parenthesized_expression,
      $.identifier,
      $.integer_literal,
      $.float_literal,
      $.boolean_literal,
      $.string_literal,
    ),

    binary_expression: $ => {
      const table = [
        ['||', 1],
        ['&&', 2],
        ['==', 3], ['!=', 3], ['<', 3], ['<=', 3], ['>', 3], ['>=', 3],
        ['+', 4], ['-', 4],
        ['*', 5], ['/', 5],
      ];
      return choice(...table.map(([op, p]) => prec.left(p, seq(
        field('left', $._expression),
        field('operator', op),
        field('right', $._expression),
      ))));
    },

    unary_expression: $ => prec(8, seq(
      field('operator', choice('-', '!')),
      field('operand', $._expression),
    )),

    cast_expression: $ => prec.left(6, seq(
      field('value', $._expression),
      'as',
      field('target', $.type),
    )),

    call_expression: $ => prec(9, seq(
      field('name', $.identifier),
      '(',
      optional(seq($._expression, repeat(seq(',', $._expression)))),
      ')',
    )),

    parenthesized_expression: $ => seq('(', $._expression, ')'),

    identifier: $ => /[a-zA-Z_][a-zA-Z0-9_]*/,

    // Float first so "1.5" doesn't tokenize as Int("1") + "." + Int("5").
    float_literal: $ => /\d+\.\d+/,
    integer_literal: $ => /\d+/,

    boolean_literal: $ => choice('true', 'false'),

    string_literal: $ => /"([^"\\]|\\.)*"/,

    line_comment: $ => token(seq('//', /[^\n]*/)),
  },
});
