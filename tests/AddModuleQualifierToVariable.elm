module Math exposing (..)

import String exposing (toInt, fromInt)

incrementString : String -> Maybe String
incrementString str = toInt str |> Maybe.map (fromInt << (+) 1)

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 6 toInt
-- INSERT String.
-- END SIMULATION


-- === expected output below ===
-- Some(QualifierAdded("toInt", "String"))
