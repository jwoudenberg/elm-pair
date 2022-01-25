module Main exposing (..)


type Age
    = Age Int


isYoung : Age -> Bool
isYoung age =
    let
        (Age ageInt) =
            age
    in
    ageInt < 30



-- START SIMULATION
-- MOVE CURSOR TO LINE 14 ageInt
-- DELETE ageInt
-- INSERT ageInYears
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- type Age
--     = Age Int
--
--
-- isYoung : Age -> Bool
-- isYoung age =
--     let
--         (Age ageInYears) =
--             age
--     in
--     ageInYears < 30
