# sim-list-cell

In one line: A classic linked-list store that holds items in order so a program can keep and change sequences.

## What it gives you

This is the everyday list keeper for SIM. It stores a run of items one after another, each cell pointing to the next, so the order you put things in is the order you get them back. Because cells can be shared, two lists can start from the same tail without copying every item, which keeps memory small when many lists overlap. You can build a list, walk it from front to back, and change what it holds while the program runs. It plugs into SIM as a loadable part, so the runtime picks it up only when a program actually asks for ordered lists.

## Why you will be glad

- Items stay in the exact order you added them.
- Shared cells let related lists reuse memory instead of copying.
- It loads on demand, so nothing pays for lists it does not use.

## Where it fits

In SIM the kernel only says what a list must do; it does not build one. This crate is one of the concrete answers to that contract, the plain in-memory choice for ordered collections. When a program needs a straightforward sequence it can grow and edit, the runtime reaches for these cells. It sits beside the lazy list backend, which serves a different need, and both speak the same shared list language so the code above them does not care which one is in play.
