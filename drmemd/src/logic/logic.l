%%

->                      "CONTROL"

\(                      "("
\)                      ")"

=                       "EQ"
>                       "GT"
>=                      "GT_EQ"
<                       "LT"
<=                      "LT_EQ"

-?[0-9]+\.[0-9]*        "FLT"
-?[0-9]+                "INT"

\{[a-z][0-9a-z_]*\}     "DEVICE"

[\t\n ]+ ;
