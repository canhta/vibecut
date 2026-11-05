// System JXA bridge: invoke AppKit's native asynchronous screen sampler.
ObjC.import('AppKit');
ObjC.import('Foundation');
var result = null;
var finished = false;
var sampler = $.NSColorSampler.alloc.init;
sampler.showSamplerWithSelectionHandler(function (color) {
    if (color && !color.isNil()) {
        var rgb = color.colorUsingColorSpace($.NSColorSpace.sRGBColorSpace);
        if (rgb && !rgb.isNil()) {
            var byte = function (v) { return Math.round(Math.min(1, Math.max(0, v)) * 255).toString(16).padStart(2, '0'); };
            result = '#' + byte(rgb.redComponent) + byte(rgb.greenComponent) + byte(rgb.blueComponent);
        }
    }
    finished = true;
});
while (!finished) {
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.05));
}
result || '';
