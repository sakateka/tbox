; extends

((table
  (dotted_key [(bare_key) (quoted_key)] @_table (_))
  (pair
    [(bare_key) (quoted_key)] @_key
    (string) @injection.content))
  (#any-of? @_table "applets" "\"applets\"" "'applets'")
  (#any-of? @_key "script" "\"script\"" "'script'")
  (#lua-match? @injection.content "^'''")
  (#offset! @injection.content 0 3 0 -3)
  (#set! injection.language "starlark")
  (#set! injection.include-children))
