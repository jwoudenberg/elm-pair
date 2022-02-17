module Main exposing (..)


booleanGrade : Int -> String
booleanGrade grade =
    case grade of
        0 ->
            "Failed :("

        1 ->
            "Passed!"

        other ->
            "Unknown grade: " ++ String.fromInt other



-- === expected output below ===
-- digraph {
-- "ArgTo((++))" -> "String" [color = red]
-- "ArgTo(CaseBranch(ResultOf(booleanGrade)).2.String.fromInt)" -> "Int" [color = red]
-- "ArgTo(ResultOf((++)))" -> "ResultOf(CaseBranch(ResultOf(booleanGrade)).2.String.fromInt)" [dir=none]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(booleanGrade)" -> "Int" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "CaseBranch(ResultOf(booleanGrade)).0" -> "String" [color = red]
-- "CaseBranch(ResultOf(booleanGrade)).1" -> "String" [color = red]
-- "CaseBranch(ResultOf(booleanGrade)).2" -> "String" [color = red]
-- "CaseBranch(ResultOf(booleanGrade)).2.other" -> "ArgTo(CaseBranch(ResultOf(booleanGrade)).2.String.fromInt)" [dir=none]
-- "CaseBranch(ResultOf(booleanGrade)).2.other" -> "CaseExpr(ResultOf(booleanGrade))" [dir=none]
-- "CaseBranch(ResultOf(booleanGrade)).2.other" -> "Int" [color = red]
-- "CaseExpr(ResultOf(booleanGrade))" -> "Int" [color = red]
-- "Int" -> "ArgTo(booleanGrade)" [dir=none]
-- "Int" -> "Int" [color = red]
-- "ResultOf(ResultOf((++)))" -> "CaseBranch(ResultOf(booleanGrade)).2" [dir=none]
-- "ResultOf(ResultOf((++)))" -> "String" [color = red]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(booleanGrade)" -> "CaseBranch(ResultOf(booleanGrade)).0" [dir=none]
-- "ResultOf(booleanGrade)" -> "CaseBranch(ResultOf(booleanGrade)).1" [dir=none]
-- "ResultOf(booleanGrade)" -> "CaseBranch(ResultOf(booleanGrade)).2" [dir=none]
-- "ResultOf(booleanGrade)" -> "String" [color = red]
-- "String" -> "ArgTo((++))" [dir=none]
-- "String" -> "CaseBranch(ResultOf(booleanGrade)).0" [dir=none]
-- "String" -> "CaseBranch(ResultOf(booleanGrade)).1" [dir=none]
-- "String" -> "ResultOf(booleanGrade)" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "booleanGrade" -> "Int -> String" [color = red]
-- "booleanGrade.grade" -> "ArgTo(booleanGrade)" [dir=none]
-- "booleanGrade.grade" -> "CaseExpr(ResultOf(booleanGrade))" [dir=none]
-- "booleanGrade.grade" -> "Int" [color = red]
-- }
