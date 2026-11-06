# Export dialog


The header Export action opens a modal with format and quality rows plus an
inline Include audio switch that is on by default. Choices use the existing
English/Vietnamese catalog and app preference store: MP4 by default, MOV or WebM;
low/medium/high/very_high quality (default high); audio included by default. The
presets request quality 55/70/85/95 and 3/6/12/24 Mb/s respectively through media's
existing encoder contract. These are explicit encoder requests, not measured
perceptual guarantees. Canvas geometry and exact frame rate default to the project. The
dialog always encodes so a selected quality is applied; the core retains its
separate Auto/copy modes.
