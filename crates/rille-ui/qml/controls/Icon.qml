import QtQuick
import QtQuick.Shapes
import rille.ui

// Line icons drawn from path data on a 24-unit grid (the project's own
// set), so they stay sharp at any size and need no image files.
Item {
    id: icon
    property string name: ""
    property color color: Theme.text
    property real size: 16
    // Stroke width in grid units.
    property real weight: 2

    implicitWidth: size
    implicitHeight: size

    // Filled shapes; everything else is stroked.
    readonly property var filled: ({ "play": true, "pause": true, "eject": true, "skip-back": true, "star": true, "dot": true })
    readonly property var paths: ({
        "chevron-right": "M9 6 L15 12 L9 18",
        "chevron-down": "M6 9 L12 15 L18 9",
        "chevron-up": "M6 15 L12 9 L18 15",
        "columns": "M4 5 H20 V19 H4 Z M9.5 5 V19 M14.5 5 V19",
        "rows": "M4 5 H20 V19 H4 Z M4 9.67 H20 M4 14.33 H20",
        "star-off": "M12 3 L14.8 8.8 L21 9.6 L16.5 14 L17.6 20.3 L12 17.3 L6.4 20.3 L7.5 14 L3 9.6 L9.2 8.8 Z",
        "chevron-left": "M15 6 L9 12 L15 18",
        "chevrons-left": "M11 6 L5 12 L11 18 M19 6 L13 12 L19 18",
        "skip-back": "M5 5 H8 V19 H5 Z M19 5 V19 L9 12 Z",
        "chevrons-right": "M13 6 L19 12 L13 18 M5 6 L11 12 L5 18",
        "folder": "M3 6.5 Q3 5 4.5 5 H9 L11 7.5 H19.5 Q21 7.5 21 9 V17.5 Q21 19 19.5 19 H4.5 Q3 19 3 17.5 Z",
        "list": "M8 6 H21 M8 12 H21 M8 18 H21 M3.5 6 H4.5 M3.5 12 H4.5 M3.5 18 H4.5",
        "suggest": "M10 3 L11.8 8.2 L17 10 L11.8 11.8 L10 17 L8.2 11.8 L3 10 L8.2 8.2 Z M18 14 L18.9 16.1 L21 17 L18.9 17.9 L18 20 L17.1 17.9 L15 17 L17.1 16.1 Z",
        "library": "M4 4 V20 M8.5 4 V20 M13 5 L17.5 19.5 M3 20 H21",
        "home": "M3 10.5 L12 3.5 L21 10.5 V20 H15 V14 H9 V20 H3 Z",
        "music": "M9 18 V5.5 L20 3.5 V16 M9 18 A3 3 0 1 1 3 18 A3 3 0 1 1 9 18 M20 16 A3 3 0 1 1 14 16 A3 3 0 1 1 20 16",
        "drive": "M3 13 H21 V19 H3 Z M3 13 L6 5 H18 L21 13 M7 16 H7.5 M11 16 H11.5",
        "computer": "M3 4 H21 V15 H3 Z M8 20 H16 M12 15 V20",
        "clock": "M12 3 A9 9 0 1 1 11.99 3 M12 7 V12 L15.5 14",
        "search": "M11 4 A7 7 0 1 1 10.99 4 M16 16 L21 21",
        "play": "M7 4 L20 12 L7 20 Z",
        "pause": "M6.5 5 H10 V19 H6.5 Z M14 5 H17.5 V19 H14 Z",
        "eject": "M5 17 H19 V19.5 H5 Z M12 4.5 L19.5 14 H4.5 Z",
        "headphones": "M3 18 V12 A9 9 0 0 1 21 12 V18 M3 14 H6.5 V20 H3 Z M17.5 14 H21 V20 H17.5 Z",
        "lock": "M5 11 H19 V21 H5 Z M8 11 V7.5 A4 4 0 0 1 16 7.5 V11",
        "sliders": "M4 21 V14 M4 10 V3 M12 21 V12 M12 8 V3 M20 21 V16 M20 12 V3 M1.5 14 H6.5 M9.5 8 H14.5 M17.5 16 H22.5",
        "x": "M6 6 L18 18 M18 6 L6 18",
        "plus": "M12 5 V19 M5 12 H19",
        "minus": "M5 12 H19",
        "square": "M5.5 5.5 H18.5 V18.5 H5.5 Z",
        "restore": "M8.5 8.5 H18.5 V18.5 H8.5 Z M5.5 15.5 V5.5 H15.5",
        "import": "M12 3 V14.5 M7 9.5 L12 14.5 L17 9.5 M4 17 V20 H20 V17",
        "analyze": "M2.5 12 H6 L9 4.5 L15 19.5 L18 12 H21.5",
        "refresh": "M20 11 A8 8 0 1 0 18 17 M20 4.5 V11 H13.5",
        "external": "M14 4 H20 V10 M20 4 L11 13 M18 14 V20 H4 V6 H10",
        "metronome": "M8 20.5 L11 3.5 H13 L16 20.5 Z M12 14 L18.5 6.5",
        "grid": "M5 4 V20 M10 4 V20 M15 4 V20 M20 4 V20",
        "zoom-in": "M11 4 A7 7 0 1 1 10.99 4 M16 16 L21 21 M8 11 H14 M11 8 V14",
        "zoom-out": "M11 4 A7 7 0 1 1 10.99 4 M16 16 L21 21 M8 11 H14",
        "maximize": "M4 9 V4 H9 M15 4 H20 V9 M20 15 V20 H15 M9 20 H4 V15",
        "midi": "M12 3 A9 9 0 1 1 11.99 3 M7.5 12.5 H8 M16 12.5 H16.5 M12 7.5 H12.5 M9 9 H9.5 M14.5 9 H15",
        "star": "M12 3 L14.8 8.8 L21 9.6 L16.5 14 L17.6 20.3 L12 17.3 L6.4 20.3 L7.5 14 L3 9.6 L9.2 8.8 Z",
        "dot": "M12 8 A4 4 0 1 1 11.99 8 Z",
        "check": "M4.5 12.5 L9.5 17.5 L19.5 6.5",
        "played": "M12 3 A9 9 0 1 1 11.99 3 M10 8.5 L15.5 12 L10 15.5 Z",
        "alert": "M12 3.5 L22 20.5 H2 Z M12 10 V14.5 M12 17.5 H12.01",
        "loop": "M17 3 L21 7 L17 11 M3 11 V9 A2 2 0 0 1 5 7 H21 M7 21 L3 17 L7 13 M21 13 V15 A2 2 0 0 1 19 17 H3",
        "trash": "M4 7 H20 M9 7 V4 H15 V7 M6 7 L7 20 H17 L18 7",
        "palette": "M12 3 A9 9 0 1 0 12 21 Q14 21 14 19 Q14 17 16 17 H18 Q21 17 21 13 A9 9 0 0 0 12 3 M7.5 11 H8 M10 7.5 H10.5 M14.5 7.5 H15",
        "cloud": "M7 19 H17.5 A4.5 4.5 0 0 0 18 10 A6.5 6.5 0 0 0 5.5 11 A4 4 0 0 0 7 19 Z",
        "eye": "M2 12 Q12 3 22 12 Q12 21 2 12 Z M12 9 A3 3 0 1 1 11.99 9",
        "eye-off": "M2 12 Q12 3 22 12 Q12 21 2 12 Z M12 9 A3 3 0 1 1 11.99 9 M4 4 L20 20",
        "drums": "M3.5 10 A8.5 3 0 1 0 20.5 10 A8.5 3 0 1 0 3.5 10 M3.5 10 V16 A8.5 3 0 0 0 20.5 16 V10 M8 3 L11 8 M17 2.5 L13.5 8"
    })

    Shape {
        anchors.fill: parent
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            scale: Qt.size(icon.size / 24, icon.size / 24)
            strokeColor: icon.filled[icon.name] ? "transparent" : icon.color
            strokeWidth: icon.weight
            fillColor: icon.filled[icon.name] ? icon.color : "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.paths[icon.name] || "" }
        }
    }
}
