module Main exposing (Meters, millis)


type Units
    = Meters
    | Seconds


type alias Meters =
    Int


millis : Meters -> Int
millis meters =
    1000 * meters



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 Meters
-- DELETE Meters
-- INSERT Distance
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (Distance, millis)
--
--
-- type Units
--     = Meters
--     | Seconds
--
--
-- type alias Distance =
--     Int
--
--
-- millis : Distance -> Int
-- millis meters =
--     1000 * meters
