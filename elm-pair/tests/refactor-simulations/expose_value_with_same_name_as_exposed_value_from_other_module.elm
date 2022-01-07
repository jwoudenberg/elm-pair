module Main exposing (..)

import Support.Date exposing (..)
import Task exposing (Task)
import Time exposing (Posix)


dateAndTime : Task e ( Date, Posix )
dateAndTime =
    Task.map
        (\time -> ( now, time ))
        Time.now



-- START SIMULATION
-- MOVE CURSOR TO LINE 5 )
-- INSERT , now
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
--         (\time -> ( Date.now, time ))
--         now
