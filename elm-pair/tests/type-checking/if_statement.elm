module Main exposing (..)


max10 : Int -> Int
max10 number =
    if number > 10 then
        10

    else
        number



-- === expected output below ===
-- digraph {
-- "(>).arg" -> "Int" [color = red]
-- "(>).result.result" -> "Bool" [color = red]
-- "(>).result.result" -> "max10.result.if_cond" [dir=none]
-- "Bool" -> "Bool" [color = red]
-- "Bool" -> "max10.result.if_cond" [dir=none]
-- "Int" -> "Int" [color = red]
-- "Int" -> "max10.arg" [dir=none]
-- "Int" -> "max10.result" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "String.length.arg" -> "String" [color = red]
-- "String.length.result" -> "Int" [color = red]
-- "max10" -> "Int -> Int" [color = red]
-- "max10.arg" -> "Int" [color = red]
-- "max10.number" -> "(>).arg" [dir=none]
-- "max10.number" -> "Int" [color = red]
-- "max10.number" -> "max10.arg" [dir=none]
-- "max10.number" -> "max10.result.if_false" [dir=none]
-- "max10.result" -> "Int" [color = red]
-- "max10.result" -> "max10.result.if_false" [dir=none]
-- "max10.result" -> "max10.result.if_true" [dir=none]
-- "max10.result.if_cond" -> "Bool" [color = red]
-- "max10.result.if_false" -> "Int" [color = red]
-- "max10.result.if_true" -> "Int" [color = red]
-- }
