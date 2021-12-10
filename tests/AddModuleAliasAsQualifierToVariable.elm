module Math exposing (..)

import String as Str exposing (toInt, fromInt)

addStrings : String -> String -> Maybe String
addStrings str1 str2 =
  Maybe.map2 (\int1 int2 -> fromInt (int1 + int2)) (toInt str1) (toInt str2)

-- START SIMULATION
-- MOVE CURSOR TO LINE 7 toInt
-- INSERT Str.
-- END SIMULATION


-- === expected output below ===
-- module Math exposing (..)
--
-- import String as Str exposing (fromInt)
--
-- addStrings : String -> String -> Maybe String
-- addStrings str1 str2 =
--   Maybe.map2 (\int1 int2 -> fromInt (int1 + int2)) (Str.toInt str1) (Str.toInt str2)
