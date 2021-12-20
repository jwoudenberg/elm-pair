module Main exposing (..)

import Time exposing (Posix, Weekday(..), millisToPosix, posixToMillis)


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
-- MOVE CURSOR TO LINE 3 Posix
-- DELETE Posix, Weekday(..), millisToPosix,
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Time exposing ( posixToMillis)
--
--
-- nextWorkDay : Time.Posix -> Time.Weekday
-- nextWorkDay time =
--     let
--         tomorrow =
--             posixToMillis time + (1000 * 3600 * 24) |> Time.millisToPosix
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
--
--
