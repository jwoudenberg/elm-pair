module Main exposing (Age, isYoung)


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
-- MOVE CURSOR TO LINE 1 Age
-- DELETE Age
-- INSERT SunLaps
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (SunLaps, isYoung)
--
--
-- type SunLaps
--     = Age Int
--
--
-- isYoung : SunLaps -> Bool
-- isYoung age =
--     let
--         (Age ageInt) =
--             age
--     in
--     ageInt < 30
