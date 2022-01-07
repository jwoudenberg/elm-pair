module Main exposing (..)

import Support.Date exposing (Date, now)
import Task exposing (Task)
import Time exposing (Posix)


dateAndTime : Task e ( Date, Posix )
dateAndTime =
    Task.map
        (\time -> ( now, time ))
        Time.now



-- START SIMULATION
-- MOVE CURSOR TO LINE 12 Time
-- DELETE Time.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Support.Date exposing (Date)
-- import Task exposing (Task)
-- import Time exposing (Posix, now)
--
--
-- dateAndTime : Task e ( Date, Posix )
-- dateAndTime =
--     Task.map
--         (\time -> ( Support.Date.now, time ))
--         now
