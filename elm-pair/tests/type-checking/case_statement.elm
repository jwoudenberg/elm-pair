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
-- "(++).arg" -> "String" [color = red]
-- "(++).result.arg" -> "(++).result.arg.fn_expr.result" [dir=none]
-- "(++).result.arg.fn_expr.arg" -> "Int" [color = red]
-- "(++).result.result" -> "String" [color = red]
-- "(++).result.result" -> "booleanGrade.result.case_branch_2" [dir=none]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "Int" [color = red]
-- "Int" -> "booleanGrade.arg" [dir=none]
-- "String" -> "(++).arg" [dir=none]
-- "String" -> "String" [color = red]
-- "String" -> "booleanGrade.result" [dir=none]
-- "String" -> "booleanGrade.result.case_branch_0" [dir=none]
-- "String" -> "booleanGrade.result.case_branch_1" [dir=none]
-- "String.length" -> "String -> Int" [color = red]
-- "String.length.arg" -> "String" [color = red]
-- "String.length.result" -> "Int" [color = red]
-- "booleanGrade" -> "Int -> String" [color = red]
-- "booleanGrade.arg" -> "Int" [color = red]
-- "booleanGrade.grade" -> "Int" [color = red]
-- "booleanGrade.grade" -> "booleanGrade.arg" [dir=none]
-- "booleanGrade.grade" -> "booleanGrade.resultcase_expr" [dir=none]
-- "booleanGrade.result" -> "String" [color = red]
-- "booleanGrade.result" -> "booleanGrade.result.case_branch_0" [dir=none]
-- "booleanGrade.result" -> "booleanGrade.result.case_branch_1" [dir=none]
-- "booleanGrade.result" -> "booleanGrade.result.case_branch_2" [dir=none]
-- "booleanGrade.result.case_branch_0" -> "String" [color = red]
-- "booleanGrade.result.case_branch_1" -> "String" [color = red]
-- "booleanGrade.result.case_branch_2" -> "String" [color = red]
-- "booleanGrade.result.case_branch_2.other" -> "(++).result.arg.fn_expr.arg" [dir=none]
-- "booleanGrade.result.case_branch_2.other" -> "Int" [color = red]
-- "booleanGrade.result.case_branch_2.other" -> "booleanGrade.resultcase_expr" [dir=none]
-- "booleanGrade.resultcase_expr" -> "Int" [color = red]
-- }
