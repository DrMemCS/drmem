%%

->                      "CONTROL"

"[^"]*"                 "STRING"

\(                      "("
\)                      ")"

not                     "B_NOT"
and                     "B_AND"
or                      "B_OR"

=                       "EQ"
\<>                      "NE"
\<=                      "LT_EQ"
>=                      "GT_EQ"
>                       "GT"
\<                       "LT"

-?[0-9]+\.[0-9]*([eE]-?[0-9]+)? "FLT"
-?[0-9]+                "INT"

\+                      "ADD"
-                       "SUB"
\*                      "MUL"
/                       "DIV"
%                       "REM"

:                       "COLON"

true                    "TRUE"
false                   "FALSE"

\{[a-z][0-9a-z_]*\}     "IDENTIFIER"

[\t\n ]+ ;

.                       "UNKNOWN"
