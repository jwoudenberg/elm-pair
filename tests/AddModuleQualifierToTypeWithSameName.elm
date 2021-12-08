-- Check no confusion arises when the name of a module and a type exposed from
-- that module are the same.

module Math exposing (..)

import Dict exposing (Dict)

timesTwo : Dict k Int -> Dict k Int
timesTwo = Dict.map (\k v -> v * 2)

-- START SIMULATION
-- MOVE CURSOR TO LINE 8 Dict
-- INSERT Dict.
-- END SIMULATION


-- === expected output below ===
-- module Math exposing (..)
--
-- import Dict
--
-- timesTwo : Dict.Dict k Int -> Dict.Dict k Int
-- timesTwo = Dict.map (\k v -> v * 2)
