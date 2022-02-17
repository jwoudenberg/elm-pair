module Main exposing (..)


combinedLength : String -> String -> Int
combinedLength strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- digraph {
-- "ArgTo((++))" -> "String" [color = red]
-- "ArgTo(ResultOf((++)))" -> "String" [color = red]
-- "ArgTo(ResultOf(combinedLength))" -> "String" [color = red]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(combinedLength)" -> "String" [color = red]
-- "ArgTo(combinedLength.String.length)" -> "ResultOf(ResultOf((++)))" [dir=none]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "Int" [color = red]
-- "Int" -> "ResultOf(ResultOf(combinedLength))" [dir=none]
-- "ResultOf(ResultOf(combinedLength))" -> "Int" [color = red]
-- "ResultOf(ResultOf(combinedLength))" -> "ResultOf(combinedLength.String.length)" [dir=none]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(combinedLength)" -> "String -> Int" [color = red]
-- "ResultOf(combinedLength.String.length)" -> "Int" [color = red]
-- "String" -> "ArgTo(ResultOf(combinedLength))" [dir=none]
-- "String" -> "ArgTo(combinedLength)" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "combinedLength" -> "String -> String -> Int" [color = red]
-- "combinedLength.strA" -> "ArgTo((++))" [dir=none]
-- "combinedLength.strA" -> "ArgTo(combinedLength)" [dir=none]
-- "combinedLength.strA" -> "String" [color = red]
-- "combinedLength.strB" -> "ArgTo(ResultOf((++)))" [dir=none]
-- "combinedLength.strB" -> "ArgTo(ResultOf(combinedLength))" [dir=none]
-- "combinedLength.strB" -> "String" [color = red]
-- }
