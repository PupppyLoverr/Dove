# Popups can't blur what's under them on Windows

zgx's popup windows (menus, popovers, tooltips) have no system backdrop on
Windows, so a `backdrop_blur` panel there is see-through without the blur.

**Workaround:** `material.popups_see_through()` (`ui/kx/material.luau`) reads
the host's `system.popups_blur` (unset, it assumes yes) and floats draw their
solid fill (`$kx.float.solid`) where they can't blur. Keplorayx's host should
share `system.popups_blur = false` on Windows until zgx gives popups Acrylic.
