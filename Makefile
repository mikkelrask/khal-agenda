PREFIX ?= $(HOME)/.local

.PHONY: install uninstall check
install:
	install -Dm755 target/release/khal-agenda $(DESTDIR)$(PREFIX)/bin/khal-agenda
	install -Dm644 data/khal-agenda.desktop $(DESTDIR)$(PREFIX)/share/applications/khal-agenda.desktop
	install -Dm644 data/khal-agenda.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/khal-agenda.svg
	install -Dm644 LICENSE $(DESTDIR)$(PREFIX)/share/licenses/khal-agenda/LICENSE
uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/khal-agenda $(DESTDIR)$(PREFIX)/share/applications/khal-agenda.desktop $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/khal-agenda.svg $(DESTDIR)$(PREFIX)/share/licenses/khal-agenda/LICENSE
check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test
	python3 -m unittest discover -s tests
