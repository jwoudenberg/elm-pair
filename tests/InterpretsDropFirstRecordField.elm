module Bulb exposing (..)

type alias Bulb =
  { kind : String
  , wattage : Int
  }

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 4 kind
-- DELETE kind : String
-- MOVE CURSOR TO LINE 5 ,
-- DELETE ,
-- END SIMULATION


-- === expected output below ===
-- Some(FieldRemoved("kind : String"))
