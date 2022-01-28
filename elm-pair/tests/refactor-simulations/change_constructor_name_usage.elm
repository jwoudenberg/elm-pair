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
-- MOVE CURSOR TO LINE 11 Age
-- DELETE Age
-- INSERT SunLaps
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (Age, isYoung)
--
--
-- type Age
--     = SunLaps Int
--
--
-- isYoung : Age -> Bool
-- isYoung age =
--     let
--         (SunLaps ageInt) =
--             age
--     in
--     ageInt < 30
