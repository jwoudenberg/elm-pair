module Main exposing (..)

{-| Module documentation here!
-}

import Task exposing (Task)


nowMillis : Task x Int
nowMillis =
    Task.succeed (Debug.todo "")



-- START SIMULATION
-- MOVE CURSOR TO LINE 11 Task
-- DELETE Task.succeed (Debug.todo "")
-- INSERT Task.map Time.posixToMillis Time.now
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- {-| Module documentation here!
-- -}
--
-- import Time
-- import Task exposing (Task)
--
--
-- nowMillis : Task x Int
-- nowMillis =
--     Task.map Time.posixToMillis Time.now
