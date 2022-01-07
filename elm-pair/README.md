# Architecture

```
            ┌──────────┐
            │programmer│
            │typing    │
            │into      ◄───────────────┐
            │editor    │               │
            └────┬─────┘               │
                 │                     │
                 │changes              │
                 │                     │refactor
             ┌───▼────┐                │
             │editor  │                │
             │listener│                │
   latest┌───┤thread  ├───┐latest      │
   code  │   └────────┘   │code        │
┌────────▼──┐             │            │
│compilation│     ┌───────┼────────────┼────┐
│thread     │     │       │            │    │
└────────┬──┘     │ ┌─────▼─┐    ┌─────┴──┐ │
         │        │ │diffing│diff│refactor│ │
     last└────────┼─►logic  ├────►engine  │ │
     compiling    │ └───────┘    └────────┘ │
     version      │      analysis thread    │
                  └─────────────────────────┘
```
