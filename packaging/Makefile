PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share

install:
	install -d $(DESTDIR)$(BINDIR)
	install -d $(DESTDIR)$(DATADIR)/applications
	install -d $(DESTDIR)$(DATADIR)/icons/hicolor/256x256/apps
	install -m 755 ../target/release/escpos_viewer $(DESTDIR)$(BINDIR)/escpos-viewer
	install -m 644 escpos-viewer.desktop $(DESTDIR)$(DATADIR)/applications/
	install -m 644 escpos-viewer.png $(DESTDIR)$(DATADIR)/icons/hicolor/256x256/apps/
	@echo "Installed escpos-viewer. Run 'escpos-viewer' to start."

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/escpos-viewer
	rm -f $(DESTDIR)$(DATADIR)/applications/escpos-viewer.desktop
	rm -f $(DESTDIR)$(DATADIR)/icons/hicolor/256x256/apps/escpos-viewer.png
	@echo "Uninstalled escpos-viewer."
