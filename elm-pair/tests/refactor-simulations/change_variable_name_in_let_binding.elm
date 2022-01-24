module Main exposing (..)


orderPizza : Int -> Int
orderPizza groupSize =
    let
        slicesPerPerson =
            2

        pizzaSize =
            6
    in
    ceiling (toFloat (groupSize * slicesPerPerson) / pizzaSize)



-- START SIMULATION
-- MOVE CURSOR TO LINE 10 pizzaSize
-- DELETE pizzaSize
-- INSERT slicesPerPizza
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
--
-- orderPizza : Int -> Int
-- orderPizza groupSize =
--     let
--         slicesPerPerson =
--             2
--
--         slicesPerPizza =
--             6
--     in
--     ceiling (toFloat (groupSize * slicesPerPerson) / slicesPerPizza)
