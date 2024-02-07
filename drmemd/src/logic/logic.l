%x VAR
%%

->                      "CONTROL"

"[^"]*"                 "STRING"

\(                      "("
\)                      ")"

not                     "B_NOT"
and                     "B_AND"
or                      "B_OR"

=                       "EQ"
\<>                     "NE"
\<=                     "LT_EQ"
>=                      "GT_EQ"
>                       "GT"
\<                      "LT"

-?[0-9]+\.[0-9]*([eE]-?[0-9]+)? "FLT"
-?[0-9]+                "INT"

\+                      "ADD"
-                       "SUB"
\*                      "MUL"
/                       "DIV"
%                       "REM"

true                    "TRUE"
false                   "FALSE"

\{                      <+VAR>"LBRACE"
<VAR>\}                 <-VAR>"RBRACE"

<VAR>[a-zA-Z][0-9a-zA-Z_]*    "IDENTIFIER"
<VAR>:                  "COLON"

[\t\n ]+ ;

.                       "UNKNOWN"
