module Bulb exposing (..)

type alias Bulb =
  { kind : String
  }

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 4 kind
-- DELETE kind : String
-- END SIMULATION


-- === expected output below ===
-- Some(FieldRemoved("kind : String"))
