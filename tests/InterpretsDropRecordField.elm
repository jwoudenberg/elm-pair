module Bulb exposing (..)

type alias Bulb =
  { kind : String
  , wattage : Int
  }

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 5 ,
-- DELETE , wattage : Int
-- END SIMULATION


-- === expected output below ===
-- Some(FieldRemoved("wattage : Int"))
