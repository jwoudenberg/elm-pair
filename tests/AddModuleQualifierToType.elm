module Math exposing (..)

import Process exposing (kill, Id)
import Task exposing (Task)

kill2 : Id -> Id -> Task x ()
kill2 proc1 proc2 =
  kill proc1
    |> Task.andThen(\_ -> kill proc2)

-- START SIMULATION
-- MOVE CURSOR TO LINE 6 Id
-- INSERT Process.
-- END SIMULATION


-- === expected output below ===
-- module Math exposing (..)
--
-- import Process exposing (kill)
-- import Task exposing (Task)
--
-- kill2 : Process.Id -> Process.Id -> Task x ()
-- kill2 proc1 proc2 =
--   kill proc1
--     |> Task.andThen(\_ -> kill proc2)
