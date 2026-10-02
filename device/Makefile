# Host builds of the device-side reader. The reader itself (ipdb/) is what gets
# compiled into the firmware; tools/ are host-only.
CC      ?= cc
CFLAGS  ?= -std=c99 -O2 -Wall -Wextra -Werror -pedantic
SAN     := -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer
LIB     ?= ../companion/target/test-library

all: ipdb_dump ipdb_fuzz

ipdb_dump: tools/ipdb_dump.c ipdb/ipdb.c ipdb/ipdb.h
	$(CC) $(CFLAGS) -Iipdb -o $@ tools/ipdb_dump.c ipdb/ipdb.c

ipdb_fuzz: tools/ipdb_fuzz.c ipdb/ipdb.c ipdb/ipdb.h
	$(CC) $(CFLAGS) $(SAN) -Iipdb -o $@ tools/ipdb_fuzz.c ipdb/ipdb.c

# Cross-compile check for the iPod's ARM926EJ-S (needs arm-none-eabi-gcc).
arm-check: ipdb/ipdb.c ipdb/ipdb.h
	arm-none-eabi-gcc -std=c99 -Os -Wall -Wextra -Werror -mcpu=arm926ej-s -marm -ffreestanding -c ipdb/ipdb.c -o /dev/null

test: all
	./ipdb_dump $(LIB) -a
	./ipdb_fuzz $(LIB)/library.ipdb 50000

clean:
	rm -f ipdb_dump ipdb_fuzz

.PHONY: all test clean arm-check
