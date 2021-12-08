module Math exposing (..)

import String exposing (toInt, fromInt)

addStrings : String -> String -> Maybe String
addStrings str1 str2 =
  Maybe.map2 (\int1 int2 -> fromInt (int1 + int2)) (toInt str1) (toInt str2)

-- START SIMULATION
-- MOVE CURSOR TO LINE 3 toInt
-- DELETE toInt,
-- END SIMULATION


-- === expected output below ===
-- module Math exposing (..)
--
-- import String exposing ( fromInt)
--
-- addStrings : String -> String -> Maybe String
-- addStrings str1 str2 =
--   Maybe.map2 (\int1 int2 -> fromInt (int1 + int2)) (String.toInt str1) (String.toInt str2)
