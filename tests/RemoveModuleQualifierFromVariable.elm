module Math exposing (..)

incrementString : String -> Maybe String
incrementString str = String.toInt str |> Maybe.map (String.fromInt << (+) 1)

-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 4 String.toInt
-- DELETE String.
-- END SIMULATION


-- === expected output below ===
-- Some(QualifierRemoved("toInt", "String"))
