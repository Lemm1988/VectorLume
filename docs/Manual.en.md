# VectorLume – Manual

VectorLume is a program for designing and lasering with GRBL-controlled diode lasers such as the Sculpfun SF-A9. You draw or import shapes, choose for each color layer how it is lasered, check the result in the preview and send the job straight to the laser.

> Safety first: wear laser safety glasses, never leave the laser unattended, provide fume extraction and fire protection, and test new settings with low power on a scrap piece first.

## Installation and first start

1. Run `VectorLume-Setup-<version>.exe`. No admin rights are needed; uninstall it via *Settings → Apps*.
2. Start VectorLume from the shortcut on the desktop or in the Start menu.
3. *Help → Load sample project* shows all processing types in one example.

Choose the language (German/English), the theme (dark/light) and the units (mm/inch) under *View*. `.vlume` projects open with a double-click; open LightBurn projects (`.lbrn2`) via *File → Open*.

Without a laser connected you can try everything with the **Simulator** port.

## Connecting the laser

1. Connect the laser via USB and switch it on. Windows reports it as a COM port. If none appears, the laser maker's USB driver is usually missing.
2. On the right, in the **Laser** tab, choose the port and click **Connect**. The **Console** shows what the controller reports.
3. *Laser → Machine settings…*: **Values from the device** takes the work area and S-max (`$30`) from the controller. Also check which corner is the origin (front left on the SF-A9).
4. VectorLume checks the laser mode. If it is off (`$32=0`), the laser stays on during travel moves. **Send $32=1** switches it on permanently.

Manage several lasers as separate devices in the machine settings. Connect network controllers (e.g. FluidNC) with the *Network* port.

## The window

- **Menu bar** at the top: *File*, *Edit*, *Arrange*, *View*, *Laser*, *Help*.
- **Tool bar** on the left: Select, Edit nodes, Trim, Place tabs, Rectangle, Ellipse, Line, Polygon, Star, Text, QR/barcode and Measure.
- **Work area** in the middle: the mouse wheel zooms; drag with the middle mouse button or Space + drag to pan. Rulers and grid show machine coordinates.
- **Right side**: the tabs *Laser*, *Move*, *Console* and *Library* at the top, the **Layers** with their settings below.
- **Status bar** at the bottom: mouse position, object count, zoom and machine state, with the layer color bar below.

## Designing

Choose a tool and drag on the work area. With the Select tool (V) you move, scale (handles on corners and edges) and rotate (rotation handle, Shift = 15° steps) shapes. Dragging a box from left to right selects fully enclosed shapes, from right to left all touched shapes.

- **Object snap** (*View → Object snap*) snaps to nodes, centers and edges; hold Alt to switch it off briefly.
- **Entering sizes**: the position and size of the selection are shown in the tool bar at the top and can be edited directly.
- **Edit nodes** (N): drag nodes and curve handles, double-click adds a node, Delete removes it.
- **Trim**: a click removes a line segment up to the next intersections.
- **Measure** (M): distance and angle between two points; the line also serves as the axis for *Mirror across line*.

## Text, QR codes and barcodes

The Text tool (T) uses all installed fonts plus built-in single-line fonts that engrave especially fast. Texts stay editable. Settings include height, letter spacing, width in percent and capitals. *Arrange → Text on path…* runs a text along a shape.

**Variable text** fills in values when lasering: `{n}` for a serial number, `{datum}` and `{zeit}` for date and time, plus columns from a CSV data list.

The QR/barcode tool (Q) creates QR codes, Data Matrix, Code 128/39, EAN-13/-8 and UPC-A, optionally inverted for anodized aluminum.

## Layers and processing types

Every shape belongs to one of 30 color layers. Click a color in the color bar at the bottom to move the selection to that layer. For each layer you set:

- **Mode**: *Line* (cut or score the outline), *Fill* (engrave the area line by line), *Offset fill* (area in paths along the outline) or *Image*.
- **Speed**, **Max/Min power** and **Passes**.
- For *Line*: kerf compensation, tabs, overcut, lead-in and perforation.
- For *Fill*: line interval, angle, crosshatch, overscan and angle step per pass.
- **Sub-layers**: process the same shapes several times, e.g. fill first, then cut the outline.

Layers can be hidden, locked and reordered. Individual objects can get their own power as a percentage of the layer power.

## Engraving images

Imported photos sit on an image layer. Choose the method:

- *Threshold*, *Floyd-Steinberg*, *Jarvis-Judice-Ninke*, *Stucki*, *Atkinson*, *Burkes*, *Sierra* and *Ordered*: dots on or off. For wood and leather, Jarvis or Stucki is a good start.
- *Grayscale*: the power follows the brightness.
- *Halftone* and *Line screen*: graphic screens with adjustable frequency and angle.
- *Sketch*: emphasizes edges like a pencil drawing.
- *3D slices*: depth over several passes.
- *Pass-through*: laser the image unchanged when it is already black and white.

Brightness, contrast, gamma, denoise and sharpen are in the image settings. *Arrange → Crop image to shape* limits an image to any shape. *Trace image…* turns an image into vector shapes, also as a centerline.

**3D models** (STL, OBJ) are imported like images. In the dialog, choose the view, the width and the resolution. VectorLume creates a depth map from the model: high areas become light, low areas dark. With the image mode *Grayscale* or *3D slices* this becomes a relief. *Invert* reverses it, e.g. for stamps and molds.

## Arranging and editing

The *Arrange* menu contains:

- **Group**, **Break apart** and **Auto-group**. The latter combines parts with their holes, e.g. after a DXF import.
- **Alignment**, **Distribute**, **Same size**, **Order**, **Dock**, **Mirror** and **Rotate**.
- **Center on work area** and **Move to bed position** (corners, edge centers, center).
- **Boolean operations**, **Weld** and **Offset**.
- **Clean up**: join open paths and delete duplicate lines.
- **Simplify shapes**, **Round corners**, **Resize slots**, **Warp** and **Stencil bridges**.
- **Nest parts**: place parts compactly on the work area or on a scrap piece.
- **Array**: as a grid, in a circle or along a path.

*Edit → Undo history…* lists the last 200 steps with what each one changed (e.g. "Moved: Circle" or "Layer changed: 3 objects"). Click an entry to restore that state. Later steps stay in the list until you change something new.

## Generators

Under *Arrange → Generators*:

- **Box generator**: boxes with finger joints from dimensions, material thickness and finger width.
- **Living hinge**: staggered cuts that make wood bendable. Select the area as a shape first.
- **Puzzle**: puzzle cuts, e.g. over an engraved photo. If something is selected when it opens, the puzzle takes its position and size.
- **Gears**: involute gears by module and number of teeth, optionally with a matching mating gear. The center distance is shown.
- **Measure kerf**: cut a test comb, measure it, apply the result to the layer (see below).

## Setting up materials

**Material test** (*Laser → Material test…*): a labeled grid of fields with stepped values, e.g. power × speed. Save the best field straight to the **Material library** and apply it to any layer later.

**Measure kerf** (*Arrange → Generators*): parts that must fit exactly, such as boxes with finger joints, need the laser's kerf on the cutting layer.

1. Create the test comb and cut it with the cutting settings you will use later.
2. Take out the strips, push them tightly together and measure across them (or measure the gap in the frame).
3. Enter the measured value and click **Apply to layer …**.

## Lasering

1. **Preview** (Alt+P): shows the finished job with run time, travel moves and a playback slider.
2. **Run frame**: the laser traces the bounding rectangle (laser off) so you can check the position on the material. *Run outline frame* traces the hull of the shapes.
3. **Start job**. Pause and stop are possible at any time; stop switches the laser off immediately.

**Start from** sets where the job goes:

- *Absolute coordinates*: as drawn on the work area.
- *User origin*: relative to a saved point.
- *Current position*: relative to the position of the laser head.

The **Origin** (9 anchor points) sets which corner of the job lies on that point. Move the head by hand in the **Move** tab. **Move laser to selection** and **Find center** help with aligning to existing workpieces.

The job settings also offer: optimize cut order, cut inner shapes before outer, start at corners and remove overlapping lines (shared edges are cut only once).

Send finished G-code files from other programs with *Laser → Send G-code file…*; save your own jobs with *File → Save G-code…*.

## Camera, Print & Cut, rotary

- **Camera** (*Laser → Camera…*): a webcam or a photo of the work area is rectified with four reference points and placed under the design.
- **Print & Cut**: align the design exactly to printed material using two printed registration marks.
- **Rotary**: rollers or chuck for glasses and mugs, connected instead of the Y motor or as an A axis.

## GRBL settings

*Laser → GRBL settings…* shows all controller settings (`$$`) with descriptions, for example max travel (`$130`, `$131`), max rate and acceleration.

1. Before any change, create a backup text file with **Back up…**.
2. Changed values have a blue outline, invalid ones a red outline.
3. **Write** sends only the changed values and then reads them back.

**Load…** takes over a backup, which you then write.

> Wrong values, e.g. for steps per mm, make the machine move incorrectly or into its end stops. If in doubt, change nothing.

## Files

- **Open and import**: VectorLume projects (`.vlume`), LightBurn projects (`.lbrn2`), SVG, DXF, PDF/AI, EPS (with Ghostscript installed), HPGL/PLT and images (PNG, JPG, BMP, GIF, WEBP, TIFF). You can also simply drag files onto the window.
- **Circuit boards**: Gerber files (copper, solder mask, silkscreen) become filled areas, the board outline becomes a cut line. Excellon drill files provide the holes as circles. If you import several files of the same board one after another, they stay aligned on top of each other. Mirror the bottom side yourself before lasering.
- **Export**: G-code, SVG, DXF and PDF (1:1 for printing).
- **Autosave**: unsaved work is backed up regularly and offered after a crash at the next start.

## Keyboard shortcuts

- Ctrl+N new, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as
- Ctrl+I import, Ctrl+E save G-code
- Ctrl+Z undo, Ctrl+Y redo
- Ctrl+C, Ctrl+X, Ctrl+V copy, cut, paste; Ctrl+D duplicate
- Ctrl+A select all, Delete delete
- Ctrl+G group, Ctrl+Shift+G ungroup
- Page Up/Page Down change order (with Ctrl to the very front/back)
- Arrow keys move by 1 mm, with Shift by 10 mm
- F or Ctrl+0 fit work area; Alt+P preview; F1 this manual
- Tools: V Select, R Rectangle, E Ellipse, L Line, T Text, Q QR/barcode, N Nodes, M Measure

## Troubleshooting

- **No port in the list**: check the USB cable and driver; some cables only charge and carry no data.
- **The laser burns lines between shapes**: laser mode is off. Send `$32=1` (VectorLume offers this when connecting).
- **Alarm after switching on**: run homing or clear the lock with `$X` in the console.
- **The job is in the wrong place**: check the origin corner in the machine settings and *Start from*; run the frame before starting.
- **Engraving too dark or too light**: run a material test; for images adjust the line interval (dpi) and image settings.
- **Parts do not fall out**: increase passes or power, check the focus, remove tabs.
- **Program error**: VectorLume writes an error report (`crash.log` in the settings folder) and shows it at the next start. Please include it when reporting bugs.
