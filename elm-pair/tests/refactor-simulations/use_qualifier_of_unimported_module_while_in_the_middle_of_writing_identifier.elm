module Main exposing (..)

import Task exposing (Task)


nowMillis : Task x Int
nowMillis =
    Task.succeed (Debug.todo "")



-- START SIMULATION
-- MOVE CURSOR TO LINE 8 Task
-- DELETE Task.succeed (Debug.todo "")
-- INSERT Parser.
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
