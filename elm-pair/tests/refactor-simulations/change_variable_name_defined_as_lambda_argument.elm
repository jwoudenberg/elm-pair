module Main exposing (..)


type Age
    = Age Int


youngest : List Age -> Maybe Age
youngest ages =
    ages
        |> List.map (\(Age n) -> n {- unpack -})
        |> List.minimum
        |> Maybe.map Age



-- START SIMULATION
-- MOVE CURSOR TO LINE 11 n {-
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
--         |> List.map (\(Age years) -> years {- unpack -})
--         |> List.minimum
--         |> Maybe.map Age