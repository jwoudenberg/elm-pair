module Main exposing (..)


foo : String -> String -> Int
foo strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- strict graph {
-- "ArgTo((++))" -- "strA"
-- "ArgTo(ResultOf((++)))" -- "strB"
-- "ArgTo(ResultOf(foo))" -- "String"
-- "ArgTo(foo)" -- "String"
-- "ResultOf(ResultOf((++)))" -- "ArgTo(String.length)"
-- "ResultOf(ResultOf(foo))" -- "Int"
-- "ResultOf(ResultOf(foo))" -- "ResultOf(String.length)"
-- "strA" -- "ArgTo(foo)"
-- "strB" -- "ArgTo(ResultOf(foo))"
-- }
