%%

->                      "CONTROL"

\(                      "("
\)                      ")"

not                     "B_NOT"
and                     "B_AND"
or                      "B_OR"

=                       "EQ"
<>                      "NE"
<=                      "LT_EQ"
>=                      "GT_EQ"
>                       "GT"
<                       "LT"

-?[0-9]+\.[0-9]*        "FLT"
-?[0-9]+                "INT"

\+                      "ADD"
-                       "SUB"
\*                      "MUL"
/                       "DIV"
%                       "REM"

true                    "TRUE"
false                   "FALSE"

\{[a-z][0-9a-z_]*\}     "DEVICE"

[\t\n ]+ ;

.                       "UNKNOWN"
