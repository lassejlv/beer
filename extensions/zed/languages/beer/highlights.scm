; Keywords
[
  "fn"
  "let"
  "use"
  "as"
] @keyword

[
  "if"
  "else"
  "while"
  "return"
] @keyword.control

; Built-in types
(type) @type.builtin

; Function definitions + calls
(function_definition
  name: (identifier) @function.definition)

(call_expression
  name: (identifier) @function.call)

; Parameters
(parameter
  name: (identifier) @variable.parameter)

; Literals
(string_literal) @string
(integer_literal) @number
(float_literal) @number
(boolean_literal) @constant.builtin

; Comments
(line_comment) @comment

; Operators
[
  "+" "-" "*" "/"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "!"
  "="
  "->"
] @operator

; Punctuation
[
  "(" ")"
  "{" "}"
  ","
  ":"
] @punctuation.bracket

; Bare identifiers (variable reads) — catch-all last so more specific
; rules above win.
(identifier) @variable
