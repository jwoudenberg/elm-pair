module Main exposing (..)


foo : String -> String -> Int
foo strA strB =
    if strA > strB then
        String.length (strA ++ strB)

    else
        42



-- === expected output below ===
-- digraph {
-- "(++)" -> "String -> String -> String" [color = red]
-- "(>)" -> "String -> String -> Bool" [color = red]
-- "ArgTo((++))" -> "String" [color = red]
-- "ArgTo((++))" -> "strA" [dir=none]
-- "ArgTo((>))" -> "String" [color = red]
-- "ArgTo((>))" -> "strA" [dir=none]
-- "ArgTo(ResultOf((++)))" -> "String" [color = red]
-- "ArgTo(ResultOf((++)))" -> "strB" [dir=none]
-- "ArgTo(ResultOf((>)))" -> "String" [color = red]
-- "ArgTo(ResultOf((>)))" -> "strB" [dir=none]
-- "ArgTo(ResultOf(foo))" -> "String" [color = red]
-- "ArgTo(ResultOf(foo))" -> "String" [dir=none]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(foo)" -> "String" [color = red]
-- "ArgTo(foo)" -> "String" [dir=none]
-- "Bool" -> "Bool" [color = red]
-- "IfCond(ResultOf(ResultOf(foo)))" -> "Bool" [color = red]
-- "IfCond(ResultOf(ResultOf(foo)))" -> "Bool" [dir=none]
-- "IfFalse(ResultOf(ResultOf(foo)))" -> "Int" [color = red]
-- "IfFalse(ResultOf(ResultOf(foo)))" -> "ResultOf(ResultOf(foo))" [dir=none]
-- "IfTrue(ResultOf(ResultOf(foo)))" -> "Int" [color = red]
-- "IfTrue(ResultOf(ResultOf(foo)))" -> "ResultOf(ResultOf(foo))" [dir=none]
-- "IfTrue(ResultOf(ResultOf(foo)))" -> "ResultOf(String.length)" [dir=none]
-- "Int" -> "Int" [color = red]
-- "ResultOf((++))" -> "String -> String" [color = red]
-- "ResultOf((>))" -> "String -> Bool" [color = red]
-- "ResultOf(ResultOf((++)))" -> "ArgTo(String.length)" [dir=none]
-- "ResultOf(ResultOf((++)))" -> "String" [color = red]
-- "ResultOf(ResultOf((>)))" -> "Bool" [color = red]
-- "ResultOf(ResultOf((>)))" -> "IfCond(ResultOf(ResultOf(foo)))" [dir=none]
-- "ResultOf(ResultOf(foo))" -> "Int" [color = red]
-- "ResultOf(ResultOf(foo))" -> "Int" [dir=none]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(foo)" -> "String -> Int" [color = red]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "foo" -> "String -> String -> Int" [color = red]
-- "strA" -> "ArgTo(foo)" [dir=none]
-- "strA" -> "String" [color = red]
-- "strB" -> "ArgTo(ResultOf(foo))" [dir=none]
-- "strB" -> "String" [color = red]
-- }
