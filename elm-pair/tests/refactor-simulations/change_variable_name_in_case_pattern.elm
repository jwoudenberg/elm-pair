module Main exposing (..)


youngest : List Int -> Int
youngest ages =
    case ages of
        [] ->
            Debug.todo ""

        [ single ] ->
            single

        head :: rest ->
            min head (youngest rest)



-- START SIMULATION
-- MOVE CURSOR TO LINE 13 rest
-- DELETE rest
-- INSERT tail
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- youngest : List Int -> Int
-- youngest ages =
--     case ages of
--         [] ->
--             Debug.todo ""
--
--         [ single ] ->
--             single
--
--         head :: tail ->
--             min head (youngest tail)
