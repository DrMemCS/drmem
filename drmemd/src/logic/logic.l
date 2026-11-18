%x VAR COLOR
%%

->                      "CONTROL"

"[^"]*"                 "STRING"

\(                      "("
\)                      ")"

[Nn][Oo][Tt]            "B_NOT"
[Aa][Nn][Dd]            "B_AND"
[Oo][Rr]                "B_OR"

[Ii][Ff]                "KW_IF"
[Tt][Hh][Ee][Nn]        "KW_THEN"
[Ee][Ll][Ss][Ee]        "KW_ELSE"
[Ee][Nn][Dd]            "KW_END"
[Cc][Oo][Aa][Ll][Ee][Ss][Cc][Ee]            "KW_COALESCE"
[Ww][Ii][Tt][Hh]_[Aa][Ll][Pp][Hh][Aa]       "KW_WITH_ALPHA"
[Bb][Rr][Ii][Gg][Hh][Tt][Nn][Ee][Ss][Ss]    "KW_BRIGHTNESS"
[Bb][Ll][Ee][Nn][Dd]                        "KW_BLEND"
,                       "COMMA"

=                       "EQ"
\<>                     "NE"
\<=                     "LT_EQ"
>=                      "GT_EQ"
>                       "GT"
\<                      "LT"

[0-9]+\.[0-9]*([eE][-+]?[0-9]+)?|\.[0-9]+([eE][-+]?[0-9]+)?|[0-9]+[eE][-+]?[0-9]+ "FLT"
2147483648              "MIN_INT"
[0-9]+                  "INT"

\+                      "ADD"
-                       "SUB"
\*                      "MUL"
/                       "DIV"
%                       "REM"

[Tt][Rr][Uu][Ee]        "TRUE"
[Ff][Aa][Ll][Ss][Ee]    "FALSE"

\{                      <+VAR>"LBRACE"
<VAR>\}                 <-VAR>"RBRACE"
<VAR>[a-zA-Z][0-9a-zA-Z_]*    "IDENTIFIER"
<VAR>:                  "COLON"

\#			<+COLOR>;
<COLOR>[0-9]+K(@[0-9]+%?)?	<-COLOR>"TEMPERATURE"
<COLOR>[0-9a-fA-F]{3,8}|[a-zA-Z][0-9a-zA-Z]*  <-COLOR>"COLOR"

[\t\n\r ]+ ;

.                       "UNKNOWN"
