# sim-list-lazy

In one line: A list that figures out its contents only when you ask, so huge or endless sequences cost little until read.

## What it gives you

This backend holds lists that stay unread until someone looks at them. Each step computes its own item and the rest of the list on demand, so a sequence can be enormous, or even without a fixed end, while only the parts you touch are ever worked out. It can also wrap a running source of values, turning a stream into a list you read one item at a time. Work is done as late as possible and no sooner, which saves effort when a program peeks at only the first few results. It loads into SIM as an optional part, present when a program wants deferred sequences.

## Why you will be glad

- Endless or very large sequences stay affordable because unread parts cost nothing.
- A live stream of values reads like an ordinary list.
- Work happens only when results are actually needed.

## Where it fits

The kernel names what a list is; it leaves the making of one to loadable parts like this. Where the cell-based backend fills a list up front, this one holds back and computes as it goes, the choice for sequences that are too big to build all at once or that arrive over time. Programs read it through the same shared list contract as every other backend, so switching to deferred behavior changes nothing in the code that consumes the list.
