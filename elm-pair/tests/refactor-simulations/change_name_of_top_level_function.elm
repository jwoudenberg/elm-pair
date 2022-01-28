module Main exposing (rateFood)


rateFood : Int -> String -> String
rateFood stars dish =
    adjective stars ++ " " ++ dish


adjective : Int -> String
adjective stars =
    case stars of
        1 ->
            "poor"

        2 ->
            "meh"

        3 ->
            "okay"

        4 ->
            "good"

        5 ->
            "amazing"

        _ ->
            "mysterious"



-- START SIMULATION
-- MOVE CURSOR TO LINE 10 adjective
-- DELETE adjective
-- INSERT ratingAdjective
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (rateFood)
--
--
-- rateFood : Int -> String -> String
-- rateFood stars dish =
--     ratingAdjective stars ++ " " ++ dish
--
--
-- ratingAdjective : Int -> String
-- ratingAdjective stars =
--     case stars of
--         1 ->
--             "poor"
--
--         2 ->
--             "meh"
--
--         3 ->
--             "okay"
--
--         4 ->
--             "good"
--
--         5 ->
--             "amazing"
--
--         _ ->
--             "mysterious"
