module Main exposing (rateFood)


rateFood : Int -> String -> String
rateFood stars dish =
    let
        adjective : Int -> String
        adjective stars_ =
            case stars_ of
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
    in
    adjective stars ++ " " ++ dish



-- START SIMULATION
-- MOVE CURSOR TO LINE 7 adjective
-- DELETE adjective
-- INSERT ratingAdjective
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (rateFood)
--
--
-- rateFood : Int -> String -> String
-- rateFood stars dish =
--     let
--         ratingAdjective : Int -> String
--         ratingAdjective stars_ =
--             case stars_ of
--                 1 ->
--                     "poor"
--
--                 2 ->
--                     "meh"
--
--                 3 ->
--                     "okay"
--
--                 4 ->
--                     "good"
--
--                 5 ->
--                     "amazing"
--
--                 _ ->
--                     "mysterious"
--     in
--     ratingAdjective stars ++ " " ++ dish
