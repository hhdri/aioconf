# Build as your user first:  cargo build --release
# Then install as root:      sudo make install
PREFIX ?= /usr/local
# Set BOARD=0 to skip loading the motherboard fan driver (nct6775) at boot.
BOARD ?= 1
BIN := target/release/aioconf

.PHONY: install uninstall

install:
	@test -x $(BIN) || { echo "Run 'cargo build --release' first."; exit 1; }
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/aioconf
	install -Dm755 contrib/aioconf-nvidia $(DESTDIR)$(PREFIX)/bin/aioconf-nvidia
	install -Dm644 contrib/aioconf.service $(DESTDIR)/etc/systemd/system/aioconf.service
	install -Dm644 contrib/60-aioconf.rules $(DESTDIR)/etc/udev/rules.d/60-aioconf.rules
	@if [ "$(BOARD)" = 1 ]; then \
		install -Dm644 contrib/modules-load.conf $(DESTDIR)/etc/modules-load.d/aioconf.conf; \
		[ -n "$(DESTDIR)" ] || modprobe nct6775 || echo "Could not load nct6775; board.pwm* curves will not apply."; \
	fi
	@if [ -e $(DESTDIR)/etc/aioconf.conf ]; then \
		echo "Keeping existing /etc/aioconf.conf"; \
	else \
		install -Dm644 contrib/aioconf.conf $(DESTDIR)/etc/aioconf.conf; \
	fi
	@if [ -z "$(DESTDIR)" ]; then \
		systemctl daemon-reload; \
		udevadm control --reload; \
		systemctl enable aioconf.service; \
		udevadm trigger --action=add --subsystem-match=hidraw; \
		systemctl restart aioconf.service; \
		echo "Installed. /etc/aioconf.conf is now applied at boot, on replug and after resume."; \
	fi

uninstall:
	-systemctl disable aioconf.service
	rm -f $(DESTDIR)$(PREFIX)/bin/aioconf $(DESTDIR)$(PREFIX)/bin/aioconf-nvidia $(DESTDIR)/etc/systemd/system/aioconf.service \
		$(DESTDIR)/etc/udev/rules.d/60-aioconf.rules $(DESTDIR)/etc/modules-load.d/aioconf.conf
	-systemctl daemon-reload
	-udevadm control --reload
	@echo "Left /etc/aioconf.conf in place. Reboot to give the fans back to the BIOS and the Kraken defaults."
