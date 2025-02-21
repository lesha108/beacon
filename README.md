# beacon
CW beacon for LibreSDR.

Program use IIO library to transmit callsing in infinite loop.
Provide freq, callsign in command line to start beacon.

Can run remotely providing host name or IP of LisreSDR or ADALM PlutoSDR. 
Release has been built for Zynq ARM platform. Can be built for intel/amd and other platforms as well.

To run on 192.168.5.10 LibreSDR:

scp beacon root@192.168.5.10:/tmp/

On LibreSDR console example:

/tmp/beacon -f 144420000 -c R2AJP

To run as generator without CW manipulation:

/tmp/beacon -f 144420000 -b

To run on another linux host:

./beacon -h 192.168.5.10 -f 144820000 -b

Short instructions to build (I use WSL Ubuntu 2204 as host platform).

Install linaro gcc toolchain compatible with linux on your Libre/Zynq/PlutoSDR. Check info page on Pluto disk.
Be warned, latest releases of gcc has incompatible with pluto linux libc version.

Install Rust.
Add Rust target armv7-unknown-linux-gnueabihf using rustup or somehow

Compile relese:

cargo build --target=armv7-unknown-linux-gnueabihf --release