module Bulb exposing (Bulb, fullName)

import String as Str

type alias Bulb =
  { kind : String
  , wattage : Int
  }

fullName : Bulb -> String
fullName { kind, wattage } = "A " ++ Str.fromInt wattage ++ "W " ++ kind ++ " bulb"

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 7 wattage
-- DELETE wattage
-- INSERT watts
-- MOVE CURSOR TO LINE 11 wattage
-- DELETE wattage
-- INSERT watts
-- MOVE CURSOR TO LINE 11 wattage
-- DELETE wattage
-- INSERT watts
-- COMPILATION SUCCEEDS
-- END SIMULATION


-- === expected output below ===
-- ()
