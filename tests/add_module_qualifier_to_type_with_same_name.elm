module Main exposing (..)

import Dict exposing (Dict)


timesTwo : Dict k Int -> Dict k Int
timesTwo =
    Dict.map (\k v -> v * 2)



-- START SIMULATION
-- MOVE CURSOR TO LINE 6 Dict
-- INSERT Dict.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Dict
--
--
-- timesTwo : Dict.Dict k Int -> Dict.Dict k Int
-- timesTwo =
--     Dict.map (\k v -> v * 2)
--
--
