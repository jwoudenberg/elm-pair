module Math exposing (..)

increment : Int -> Int
increment int = int + 1

plusTwo : Int -> Int
plusTwo int = increment (increment int)


-- START SIMULATION
-- COMPILATION SUCCEEDS
-- MOVE CURSOR TO LINE 7 increment
-- DELETE increment
-- INSERT inc
-- END SIMULATION


-- === expected output below ===
-- Some(NameChanged("increment", "inc"))
