module Main exposing (..)


ages2 : ()
ages2 =
    ()


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
-- MOVE CURSOR TO LINE 18 rest
-- DELETE rest
-- INSERT ages
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- ages2 : ()
-- ages2 =
--     ()
--
--
-- youngest : List Int -> Int
-- youngest ages3 =
--     case ages3 of
--         [] ->
--             Debug.todo ""
--
--         [ single ] ->
--             single
--
--         head :: ages ->
--             min head (youngest ages)
