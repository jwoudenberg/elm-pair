module Bulb exposing (..)

type Bulb = Incandescent | Cfl | Halogen | Led

type alias Wattage = Int

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 3
-- DELETE type Bulb = Incandescent | Cfl | Halogen | Led
-- END SIMULATION


-- === expected output below ===
-- Some(TypeRemoved("type Bulb = Incandescent | Cfl | Halogen | Led"))
