module Main exposing (..)


combinedLength : String -> String -> Int
combinedLength strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- digraph {
-- "(++)" -> "String -> String -> String" [color = red]
-- "(++).arg" -> "String" [color = red]
-- "(++).result" -> "String -> String" [color = red]
-- "(++).result.arg" -> "String" [color = red]
-- "(++).result.result" -> "String" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "Int" [color = red]
-- "Int" -> "combinedLength.result.result" [dir=none]
-- "String" -> "String" [color = red]
-- "String" -> "combinedLength.arg" [dir=none]
-- "String" -> "combinedLength.result.arg" [dir=none]
-- "String.length" -> "String -> Int" [color = red]
-- "String.length" -> "combinedLength.result.result.fn_expr" [dir=none]
-- "String.length.arg" -> "String" [color = red]
-- "String.length.result" -> "Int" [color = red]
-- "combinedLength" -> "String -> String -> Int" [color = red]
-- "combinedLength.arg" -> "String" [color = red]
-- "combinedLength.result" -> "String -> Int" [color = red]
-- "combinedLength.result.arg" -> "String" [color = red]
-- "combinedLength.result.result" -> "Int" [color = red]
-- "combinedLength.result.result" -> "combinedLength.result.result.fn_expr.result" [dir=none]
-- "combinedLength.result.result.fn_expr" -> "String -> Int" [color = red]
-- "combinedLength.result.result.fn_expr.arg" -> "(++).result.result" [dir=none]
-- "combinedLength.result.result.fn_expr.arg" -> "String" [color = red]
-- "combinedLength.result.result.fn_expr.result" -> "Int" [color = red]
-- "combinedLength.strA" -> "(++).arg" [dir=none]
-- "combinedLength.strA" -> "String" [color = red]
-- "combinedLength.strA" -> "combinedLength.arg" [dir=none]
-- "combinedLength.strB" -> "(++).result.arg" [dir=none]
-- "combinedLength.strB" -> "String" [color = red]
-- "combinedLength.strB" -> "combinedLength.result.arg" [dir=none]
-- }
