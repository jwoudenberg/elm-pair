module Main exposing (..)

import Time exposing (..)


nextWorkDay : Posix -> Weekday
nextWorkDay time =
    let
        tomorrow =
            posixToMillis time + (1000 * 3600 * 24) |> millisToPosix
    in
    case Time.toWeekday Time.utc tomorrow of
        Sat ->
            Mon

        Sun ->
            Mon

        day ->
            day



-- START SIMULATION
-- MOVE CURSOR TO LINE 14 Mon
-- INSERT Time.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Time exposing (..)
--
--
-- nextWorkDay : Posix -> Weekday
-- nextWorkDay time =
--     let
--         tomorrow =
--             posixToMillis time + (1000 * 3600 * 24) |> millisToPosix
--     in
--     case Time.toWeekday Time.utc tomorrow of
--         Time.Sat ->
--             Time.Mon
--
--         Time.Sun ->
--             Time.Mon
--
--         day ->
--             day
