# Host builds of the device-side reader. The reader itself (ipdb/) is what gets
# compiled into the firmware; tools/ are host-only.
CC      ?= cc
CFLAGS  ?= -std=c99 -O2 -Wall -Wextra -Werror -pedantic
SAN     := -O1 -g -fsanitize=address,undefined -fno-sanitize-recover=all -fno-omit-frame-pointer
LIB     ?= ../companion/target/test-library
FONTS   ?= ../companion/target/test-fonts

all: ipdb_dump ipdb_fuzz link_host ipfn_test

ipdb_dump: tools/ipdb_dump.c ipdb/ipdb.c ipdb/ipdb.h
	$(CC) $(CFLAGS) -Iipdb -o $@ tools/ipdb_dump.c ipdb/ipdb.c

ipdb_fuzz: tools/ipdb_fuzz.c ipdb/ipdb.c ipdb/ipdb.h
	$(CC) $(CFLAGS) $(SAN) -Iipdb -o $@ tools/ipdb_fuzz.c ipdb/ipdb.c

link_host: tools/link_host.c link/link_proto.c link/link_proto.h ipdb/ipdb.c
	$(CC) $(CFLAGS) $(SAN) -D_POSIX_C_SOURCE=200809L -Iipdb -Ilink -o $@ tools/link_host.c link/link_proto.c ipdb/ipdb.c

ipfn_test: tools/ipfn_test.c font/ipfn.c font/ipfn.h ipdb/ipdb.c
	$(CC) $(CFLAGS) $(SAN) -D_POSIX_C_SOURCE=200809L -Iipdb -Ifont -o $@ tools/ipfn_test.c font/ipfn.c ipdb/ipdb.c

# Cross-compile check for the iPod's ARM926EJ-S (needs arm-none-eabi-gcc).
arm-check: ipdb/ipdb.c ipdb/ipdb.h
	arm-none-eabi-gcc -std=c99 -Os -Wall -Wextra -Werror -mcpu=arm926ej-s -marm -ffreestanding -c ipdb/ipdb.c -o /dev/null
	arm-none-eabi-gcc -std=c99 -Os -Wall -Wextra -Werror -mcpu=arm926ej-s -marm -ffreestanding -Iipdb -c link/link_proto.c -o /dev/null
	arm-none-eabi-gcc -std=c99 -Os -Wall -Wextra -Werror -mcpu=arm926ej-s -marm -ffreestanding -Iipdb -c font/ipfn.c -o /dev/null

test: all
	./ipdb_dump $(LIB) -a
	./ipdb_fuzz $(LIB)/library.ipdb 50000
	node ../companion/web/test-link.mjs ./link_host
	@for f in $(FONTS)/*.ipfn; do ./ipfn_test $$f || exit 1; done

clean:
	rm -f ipdb_dump ipdb_fuzz link_host ipfn_test

.PHONY: all test clean arm-check
