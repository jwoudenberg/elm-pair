module Main exposing (..)

import Support.Date as Date exposing (Clock)


type Delay
    = Minutes Int
    | Hours Int
    | Days Int


minutesLate : Int -> Delay
minutesLate minutes =
    Minutes minutes



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 )
-- INSERT , Minutes
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
