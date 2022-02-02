module Main exposing (..)


type Age
    = Age Int


birthYear : Age -> { r | year : Int } -> Int
birthYear (Age ageInYears) { year } =
    year - ageInYears



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 year
-- DELETE year
-- INSERT current_year
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
