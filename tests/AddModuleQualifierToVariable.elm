module Math exposing (..)

import String exposing (toInt, fromInt)

incrementString : String -> Maybe String
incrementString str = toInt str |> Maybe.map (fromInt << (+) 1)

-- START SIMULATION
-- MOVE CURSOR TO LINE 6 toInt
-- INSERT String.
-- END SIMULATION


-- === expected output below ===
-- module Math exposing (..)
-- 
-- import String exposing (fromInt)
-- 
-- incrementString : String -> Maybe String
-- incrementString str = String.toInt str |> Maybe.map (fromInt << (+) 1)
