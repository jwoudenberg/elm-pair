module Main exposing (..)


type Age
    = Age Int


youngest : List Age -> Maybe Age
youngest ages =
    ages
        |> List.map (\(Age n) -> n)
        |> List.minimum
        |> Maybe.map Age


idAge : Age -> Age
idAge years =
    years



-- START SIMULATION
-- MOVE CURSOR TO LINE 11 n)
-- DELETE n
-- INSERT years
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- type Age
--     = Age Int
--
--
-- youngest : List Age -> Maybe Age
-- youngest ages =
--     ages
--         |> List.map (\(Age years) -> years)
--         |> List.minimum
--         |> Maybe.map Age
--
--
-- idAge : Age -> Age
-- idAge years =
--     years
--
--
