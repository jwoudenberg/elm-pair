module Math exposing (..)

increment : Int -> Int
increment int = int + 1


-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 4 int +
-- DELETE int
-- INSERT number
-- END SIMULATION


-- === expected output below ===
-- Some(NameChanged("int", "number"))
