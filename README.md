# Serial Experiments GBA

An experiment for interacting with the Gameboy Advance's serial link cable
functionality in Rust. Built on top of the
[`agb`](https://github.com/agbrs/agb/tree/master) library and attempts to
emulate the ownership style used there.

## Functionality

Right now focus is being given to provide an easy-to-use-interface for the GBA's
"multiplayer mode", which is the standard 4-player link session used for
multiplayer games. In addition code has been written to support the GBA's GPIO
mode but it has not yet been tested. 