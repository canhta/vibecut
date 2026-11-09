# Export dialog

The header Export action opens a modal with format and quality rows and an Include audio switch. Defaults: MP4 (or MOV, WebM), quality high, audio on, project canvas and exact frame rate. Quality low/medium/high/very_high requests encoder quality 55/70/85/95 and 3/6/12/24 Mb/s; these are encoder requests, not perceptual guarantees. The dialog always encodes; the core keeps its separate Auto/copy modes. Choices persist in the app preference store. Settings and publication: [export](../../export/README.md).
