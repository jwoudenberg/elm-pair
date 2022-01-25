module Main exposing (..)


isYoung : Int -> Int -> Bool
isYoung birthYear_ currentYear =
    let
        age =
            currentYear - birthYear_
    in
    age < 30


birthYear : Int -> Int -> Int
birthYear age currentYear =
    currentYear - age



-- START SIMULATION
-- MOVE CURSOR TO LINE 14 age
-- DELETE age
-- INSERT ageInYears
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- isYoung : Int -> Int -> Bool
-- isYoung birthYear_ currentYear =
--     let
--         age =
--             currentYear - birthYear_
--     in
--     age < 30
--
--
-- birthYear : Int -> Int -> Int
-- birthYear ageInYears currentYear =
--     currentYear - ageInYears
