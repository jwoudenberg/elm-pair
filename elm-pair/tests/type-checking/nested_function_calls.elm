module Main exposing (..)


combinedLength : String -> String -> Int
combinedLength strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- digraph {
-- "(++)" -> "String -> String -> String" [color = red]
-- "ArgTo((++))" -> "String" [color = red]
-- "ArgTo(FnExpr(ResultOf(ResultOf(combinedLength))))" -> "ResultOf(ResultOf((++)))" [dir=none]
-- "ArgTo(FnExpr(ResultOf(ResultOf(combinedLength))))" -> "String" [color = red]
-- "ArgTo(ResultOf((++)))" -> "String" [color = red]
-- "ArgTo(ResultOf(combinedLength))" -> "String" [color = red]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(combinedLength)" -> "String" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "FnExpr(ResultOf(ResultOf(combinedLength)))" -> "String -> Int" [color = red]
-- "Int" -> "Int" [color = red]
-- "Int" -> "ResultOf(ResultOf(combinedLength))" [dir=none]
-- "ResultOf((++))" -> "String -> String" [color = red]
-- "ResultOf(FnExpr(ResultOf(ResultOf(combinedLength))))" -> "Int" [color = red]
-- "ResultOf(ResultOf((++)))" -> "String" [color = red]
-- "ResultOf(ResultOf(combinedLength))" -> "Int" [color = red]
-- "ResultOf(ResultOf(combinedLength))" -> "ResultOf(FnExpr(ResultOf(ResultOf(combinedLength))))" [dir=none]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(combinedLength)" -> "String -> Int" [color = red]
-- "String" -> "ArgTo(ResultOf(combinedLength))" [dir=none]
-- "String" -> "ArgTo(combinedLength)" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "FnExpr(ResultOf(ResultOf(combinedLength)))" [dir=none]
-- "String.length" -> "String -> Int" [color = red]
-- "combinedLength" -> "String -> String -> Int" [color = red]
-- "combinedLength.strA" -> "ArgTo((++))" [dir=none]
-- "combinedLength.strA" -> "ArgTo(combinedLength)" [dir=none]
-- "combinedLength.strA" -> "String" [color = red]
-- "combinedLength.strB" -> "ArgTo(ResultOf((++)))" [dir=none]
-- "combinedLength.strB" -> "ArgTo(ResultOf(combinedLength))" [dir=none]
-- "combinedLength.strB" -> "String" [color = red]
-- }
