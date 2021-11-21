module Math exposing (..)

increment : Int -> Int
increment int =
    let incremented = int + 1
    in incremented


-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 5 incremented
-- DELETE incremented
-- INSERT result
-- END SIMULATION


-- === expected output below ===
-- Some(NameChanged("incremented", "result"))
