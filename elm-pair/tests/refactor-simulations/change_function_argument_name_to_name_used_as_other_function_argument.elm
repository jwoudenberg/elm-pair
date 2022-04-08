module Main exposing (..)


birthYear : Int -> Int -> Int
birthYear age currentYear =
    currentYear - age


isYoung : Int -> Bool
isYoung ageInYears =
    ageInYears < 30



-- START SIMULATION
-- MOVE CURSOR TO LINE 10 ageInYears
-- DELETE ageInYears
-- INSERT age
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- birthYear : Int -> Int -> Int
-- birthYear age currentYear =
--     currentYear - age
--
--
-- isYoung : Int -> Bool
-- isYoung age =
--     age < 30
--
