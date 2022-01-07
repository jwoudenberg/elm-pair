module Main exposing (..)

import Time exposing (Weekday)


type WeekendDay
    = Sat
    | Sun


toWeekday : WeekendDay -> Weekday
toWeekday day =
    case day of
        Sat ->
            Time.Sat

        Sun ->
            Time.Sun



-- START SIMULATION
-- MOVE CURSOR TO LINE 15 Time
-- DELETE Time.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Time exposing (Weekday(..))
--
--
-- type WeekendDay
--     = Sat2
--     | Sun2
--
--
-- toWeekday : WeekendDay -> Weekday
-- toWeekday day =
--     case day of
--         Sat2 ->
--             Sat
--
--         Sun2 ->
--             Sun
