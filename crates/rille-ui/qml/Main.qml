pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import rille.ui

ApplicationWindow {
    id: window
    width: 1440
    height: 900
    // Down to phone size: narrow windows switch to the mobile layout.
    minimumWidth: 360
    minimumHeight: 320
    visible: true
    // No system frame (GNOME would draw a light one): the header bar is the
    // title bar, the edges below resize.
    flags: Qt.Window | Qt.FramelessWindowHint
    title: "rille"
    // Item the screenshot option saves instead of the window (e.g. a menu).
    property Item grabItem: null
    readonly property bool maximized: visibility === Window.Maximized || visibility === Window.FullScreen

    function toggleMaximized() {
        if (visibility === Window.Maximized)
            showNormal()
        else
            showMaximized()
    }
    color: Theme.bg
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fontNormal

    DeckController { id: deckA; deck: 0 }
    DeckController { id: deckB; deck: 1 }
    DeckController { id: deckC; deck: 2 }
    DeckController { id: deckD; deck: 3 }

    // Settings → Decks: A/C stacked on the left, B/D on the right.
    readonly property bool fourDecks: AppController.deckCount === 4
    readonly property int mixerWidth: fourDecks ? Theme.mixerWidth4 : Theme.mixerWidth
    // Settings → Decks → Deck height, for each deck (twice per side with 4).
    readonly property int deckRowExtra: (Theme.deckHeightExtra[AppController.deckHeight] || 0) * (fourDecks ? 2 : 1)
    // Settings → Decks → Drum machine: shown above or below the decks.
    readonly property int drumsHeight: AppController.drumsRows === 4 ? Theme.drumsHeight4 : Theme.drumsHeight1
    readonly property int drumsRoom: AppController.drumsVisible ? drumsHeight + Theme.gap : 0

    // Mobile layout: one view at a time (`tab`) behind a tab bar, for
    // phones and windows too small for everything at once. `--mobile` and
    // `--desktop` force a layout.
    readonly property bool mobile: AppController.hasArg("mobile")
        || (!AppController.hasArg("desktop") && Theme.isMobileSize(width, height))
    readonly property bool landscape: width > height
    property int tab: 0 // decks, mixer, drums, library
    // With 4 decks, the pair on the decks view: A/B or C/D.
    property int deckPair: 0
    onMobileChanged: Theme.mobile = mobile
    function deckShown(d: int): bool {
        return !mobile || !fourDecks || Math.floor(d / 2) === deckPair
    }

    // One state refresh per frame.
    FrameAnimation {
        running: true
        onTriggered: {
            AppController.tick()
            deckA.refresh()
            deckB.refresh()
            if (window.fourDecks) {
                deckC.refresh()
                deckD.refresh()
            }
        }
    }

    // Keyboard: deck A on the left of the keyboard, deck B on the right.
    // Keys are ignored while typing in a text field.
    readonly property var keyMap: ({
        "Z": "deck.A.play", "X": "deck.A.cue", "C": "deck.A.sync", "S": "deck.A.loop_toggle",
        "Q": "deck.A.beatjump_back", "W": "deck.A.beatjump_forward",
        "1": "deck.A.hotcue.1", "2": "deck.A.hotcue.2", "3": "deck.A.hotcue.3", "4": "deck.A.hotcue.4",
        "M": "deck.B.play", "N": "deck.B.cue", "B": "deck.B.sync", "K": "deck.B.loop_toggle",
        "O": "deck.B.beatjump_back", "P": "deck.B.beatjump_forward",
        "7": "deck.B.hotcue.1", "8": "deck.B.hotcue.2", "9": "deck.B.hotcue.3", "0": "deck.B.hotcue.4"
    })
    // Shift + a deck's CUE key: back to the track start.
    readonly property var shiftKeyMap: ({ "X": "deck.A.jump_start", "N": "deck.B.jump_start" })
    function keyTarget(event) {
        if (activeFocusItem && activeFocusItem instanceof TextField)
            return ""
        if (event.modifiers & (Qt.ControlModifier | Qt.AltModifier))
            return ""
        var key = event.text.toUpperCase()
        if (event.modifiers & Qt.ShiftModifier)
            return shiftKeyMap[key] || ""
        return keyMap[key] || ""
    }

    Item {
        anchors.fill: parent
        focus: true
        Keys.onPressed: event => {
            // Page keys scroll the tracklist wherever the focus is.
            if (event.key === Qt.Key_PageUp || event.key === Qt.Key_PageDown) {
                browserView.pageRows(event.key === Qt.Key_PageUp ? -1 : 1, event.modifiers & Qt.ShiftModifier)
                event.accepted = true
                return
            }
            var t = window.keyTarget(event)
            if (t.length && !event.isAutoRepeat) {
                AppController.press(t, true)
                event.accepted = true
            }
        }
        Keys.onReleased: event => {
            var t = window.keyTarget(event)
            if (t.length && !event.isAutoRepeat) {
                AppController.press(t, false)
                event.accepted = true
            }
        }

        ColumnLayout {
            id: rootLayout
            anchors.fill: parent
            anchors.margins: Theme.gap
            spacing: Theme.gap

            HeaderBar {
                compact: window.mobile
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.headerHeight
                onSettingsRequested: settings.open()
                onAboutRequested: about.open()
                onFullscreenRequested: window.visibility = window.visibility === Window.FullScreen ? Window.Windowed : Window.FullScreen
                maximized: window.maximized
                onMoveRequested: window.startSystemMove()
                onMinimizeRequested: window.showMinimized()
                onMaximizeRequested: window.toggleMaximized()
                onCloseRequested: window.close()
            }

            RowLayout {
                visible: !window.mobile
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.topRowHeight
                Layout.fillHeight: false
                spacing: Theme.gap
                // Nested layouts default to fill; equal preferred widths split the space evenly.
                FxUnitPanel { unitIndex: 0; Layout.preferredWidth: 1; Layout.fillWidth: true; Layout.fillHeight: true }
                MasterClockPanel { Layout.preferredWidth: window.mixerWidth; Layout.fillHeight: true }
                FxUnitPanel { unitIndex: 1; Layout.preferredWidth: 1; Layout.fillWidth: true; Layout.fillHeight: true }
            }

            // The drum machine, above the decks (only one panel exists);
            // the drums view in the mobile layout.
            Loader {
                active: window.mobile ? window.tab === 2 : (AppController.drumsVisible && AppController.drumsPosition === 0)
                visible: active
                sourceComponent: drumPanel
                Layout.fillWidth: true
                Layout.preferredHeight: window.mobile ? -1 : window.drumsHeight
                Layout.fillHeight: window.mobile
            }

            // Mobile, 4 decks: which pair the decks view shows.
            RowLayout {
                visible: window.mobile && window.tab === 0 && window.fourDecks
                Layout.fillWidth: true
                Layout.fillHeight: false
                spacing: Theme.gap
                Repeater {
                    model: ["A · B", "C · D"]
                    DjButton {
                        required property string modelData
                        required property int index
                        Layout.fillWidth: true
                        implicitHeight: 36
                        text: modelData
                        lit: window.deckPair === index
                        litColor: Theme.sync
                        onClicked: window.deckPair = index
                    }
                }
            }

            // Mobile: the decks view, decks stacked in portrait, side by
            // side in landscape (no mixer between them).
            GridLayout {
                id: deckArea
                visible: !window.mobile || window.tab === 0
                readonly property bool stacked: window.mobile && !window.landscape
                // Short decks on a small screen: the compact deck layout.
                readonly property bool tight: (deckArea.stacked ? (deckArea.height - Theme.gap) / 2 : deckArea.height) < 280
                Layout.fillWidth: true
                rowSpacing: Theme.gap
                columnSpacing: Theme.gap
                // 4 decks take about half the window, never less than 2 decks.
                // Taller decks grow as far as the browser keeps its minimum.
                readonly property int baseHeight: window.fourDecks
                    ? Math.max(Theme.deckRowHeight, Math.min(Theme.deckRowHeight4, Math.round(window.height * 0.49)))
                    : Theme.deckRowHeight
                readonly property int room: rootLayout.height - Theme.headerHeight - Theme.topRowHeight - Theme.stripHeight
                    - 4 * Theme.gap - Theme.browserMinHeight - window.drumsRoom
                Layout.preferredHeight: window.mobile ? -1 : Math.max(baseHeight, Math.min(baseHeight + window.deckRowExtra, room))
                Layout.fillHeight: window.mobile
                ColumnLayout {
                    Layout.row: 0
                    Layout.column: 0
                    Layout.preferredWidth: 1
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: Theme.gap
                    Deck { dc: deckA; compact: window.mobile ? deckArea.tight : window.fourDecks; visible: !deckA.remix && window.deckShown(0); Layout.fillWidth: true; Layout.fillHeight: true }
                    RemixDeck { dc: deckA; compact: window.mobile ? deckArea.tight : window.fourDecks; visible: deckA.remix && window.deckShown(0); Layout.fillWidth: true; Layout.fillHeight: true }
                    Deck { dc: deckC; compact: window.mobile ? deckArea.tight : true; visible: window.fourDecks && !deckC.remix && window.deckShown(2); Layout.fillWidth: true; Layout.fillHeight: true }
                    RemixDeck { dc: deckC; compact: window.mobile ? deckArea.tight : true; visible: window.fourDecks && deckC.remix && window.deckShown(2); Layout.fillWidth: true; Layout.fillHeight: true }
                }
                // Settings → Audio: hidden when a controller does the mixing.
                Mixer {
                    visible: !AppController.mixerHidden && !window.mobile
                    Layout.row: 0
                    Layout.column: 1
                    deckA: deckA
                    deckB: deckB
                    deckC: deckC
                    deckD: deckD
                    fourDecks: window.fourDecks
                    Layout.preferredWidth: window.mixerWidth
                    Layout.fillWidth: false
                    Layout.fillHeight: true
                }
                ColumnLayout {
                    Layout.row: deckArea.stacked ? 1 : 0
                    Layout.column: deckArea.stacked ? 0 : 2
                    Layout.preferredWidth: 1
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    spacing: Theme.gap
                    Deck { dc: deckB; compact: window.mobile ? deckArea.tight : window.fourDecks; visible: !deckB.remix && window.deckShown(1); Layout.fillWidth: true; Layout.fillHeight: true }
                    RemixDeck { dc: deckB; compact: window.mobile ? deckArea.tight : window.fourDecks; visible: deckB.remix && window.deckShown(1); Layout.fillWidth: true; Layout.fillHeight: true }
                    Deck { dc: deckD; compact: window.mobile ? deckArea.tight : true; visible: window.fourDecks && !deckD.remix && window.deckShown(3); Layout.fillWidth: true; Layout.fillHeight: true }
                    RemixDeck { dc: deckD; compact: window.mobile ? deckArea.tight : true; visible: window.fourDecks && deckD.remix && window.deckShown(3); Layout.fillWidth: true; Layout.fillHeight: true }
                }
            }

            // A hardware mixer has the crossfader and headphones.
            CrossfaderStrip {
                visible: AppController.mixerChannels.length === 0 && !AppController.mixerHidden && (!window.mobile || window.tab === 0)
                compact: window.mobile
                Layout.fillWidth: true
                Layout.preferredHeight: window.mobile ? Theme.stripHeight + 6 : Theme.stripHeight
                mixerWidth: window.mixerWidth
                fourDecks: window.fourDecks
            }

            // …or below them.
            Loader {
                active: !window.mobile && AppController.drumsVisible && AppController.drumsPosition === 1
                visible: active
                sourceComponent: drumPanel
                Layout.fillWidth: true
                Layout.preferredHeight: window.drumsHeight
                Layout.fillHeight: false
            }

            Browser {
                id: browserView
                // Hidden, not unloaded, in the mobile layout: keeps its place.
                visible: !window.mobile || window.tab === 3
                Layout.fillWidth: true
                Layout.fillHeight: true
                onSettingsRequested: page => {
                    settings.page = page
                    settings.open()
                }
            }

            Loader {
                active: window.mobile && window.tab === 1
                visible: active
                Layout.fillWidth: true
                Layout.fillHeight: true
                sourceComponent: MobileMixerPage {
                    deckA: deckA
                    deckB: deckB
                    deckC: deckC
                    deckD: deckD
                    fourDecks: window.fourDecks
                }
            }

            MiniDeckStrip {
                visible: window.mobile && window.tab !== 0
                decks: window.fourDecks ? [deckA, deckB, deckC, deckD] : [deckA, deckB]
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.miniStripHeight
                onDeckRequested: d => {
                    window.tab = 0
                    window.deckPair = Math.floor(d / 2)
                }
            }

            MobileTabBar {
                visible: window.mobile
                current: window.tab
                drumsPlaying: AppController.drumsPlaying
                beat: AppController.clockBeat
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.tabBarHeight
                onSelected: i => window.tab = i
            }
        }
    }

    // Resize handles along the edges and corners of the frameless window.
    Repeater {
        model: [
            { e: Qt.LeftEdge, c: Qt.SizeHorCursor }, { e: Qt.RightEdge, c: Qt.SizeHorCursor },
            { e: Qt.TopEdge, c: Qt.SizeVerCursor }, { e: Qt.BottomEdge, c: Qt.SizeVerCursor },
            { e: Qt.TopEdge | Qt.LeftEdge, c: Qt.SizeFDiagCursor }, { e: Qt.BottomEdge | Qt.RightEdge, c: Qt.SizeFDiagCursor },
            { e: Qt.TopEdge | Qt.RightEdge, c: Qt.SizeBDiagCursor }, { e: Qt.BottomEdge | Qt.LeftEdge, c: Qt.SizeBDiagCursor }
        ]
        MouseArea {
            id: edge
            required property var modelData
            readonly property int grip: 5
            readonly property bool atLeft: (edge.modelData.e & Qt.LeftEdge) !== 0
            readonly property bool atRight: (edge.modelData.e & Qt.RightEdge) !== 0
            readonly property bool atTop: (edge.modelData.e & Qt.TopEdge) !== 0
            readonly property bool atBottom: (edge.modelData.e & Qt.BottomEdge) !== 0
            visible: !window.maximized
            z: 100
            x: edge.atRight ? window.width - (edge.atTop || edge.atBottom ? 12 : edge.grip) : 0
            y: edge.atBottom ? window.height - (edge.atLeft || edge.atRight ? 12 : edge.grip) : 0
            width: edge.atLeft || edge.atRight ? (edge.atTop || edge.atBottom ? 12 : edge.grip) : window.width
            height: edge.atTop || edge.atBottom ? (edge.atLeft || edge.atRight ? 12 : edge.grip) : window.height
            cursorShape: edge.modelData.c
            onPressed: window.startSystemResize(edge.modelData.e)
        }
    }

    Component {
        id: drumPanel
        DrumMachine {}
    }

    SettingsDialog { id: settings }
    AboutDialog { id: about }

    // First start: no music folder yet.
    Component.onCompleted: {
        Theme.mobile = window.mobile
        // `--tab-mixer`, `--tab-drums`, `--tab-library`: start on a view of
        // the mobile layout (screenshots).
        var tabs = ["mixer", "drums", "library"]
        for (var i = 0; i < tabs.length; i++) {
            if (AppController.hasArg("tab-" + tabs[i]))
                window.tab = i + 1
        }
        // `--settings` (or `--settings1`…`--settings4` for a page) opens the settings.
        for (var p = 0; p < settings.pages.length; p++) {
            if (AppController.hasArg(p === 0 ? "settings" : "settings" + p)) {
                settings.page = p
                settings.open()
            }
        }
        if (AppController.hasArg("about"))
            about.open()
        var size = AppController.windowSize().split("x")
        if (size.length === 2 && Number(size[0]) > 0 && Number(size[1]) > 0) {
            window.width = Number(size[0])
            window.height = Number(size[1])
        }
        var s = JSON.parse(AppController.settingsJson())
        if ((!s.library_roots || s.library_roots.length === 0) && AppController.trackCount() === 0 && !AppController.smokeTest() && AppController.screenshotPath() === "") {
            settings.open()
            AppController.status = "Welcome! Add your music folder under Settings → Library."
        }
    }

    // `rille --smoke-test`: render briefly, walk through the mobile layout
    // (portrait, every view, landscape) and back, then quit. CI runs this
    // offscreen with QT_FATAL_WARNINGS=1 so any QML warning fails the build.
    // `rille --screenshot=<file.png>`: save one frame, then quit.
    Timer {
        running: AppController.smokeTest() || AppController.screenshotPath() !== ""
        interval: AppController.screenshotPath() !== "" ? AppController.screenshotDelay() : 1500
        onTriggered: {
            var path = AppController.screenshotPath()
            if (path === "") {
                smokeSteps.start()
                return
            }
            (window.grabItem ? window.grabItem : (settings.opened ? settings.background.parent : (about.opened ? about.background.parent : rootLayout))).grabToImage(result => Qt.exit(result.saveToFile(path) ? 0 : 1))
        }
    }
    Timer {
        id: smokeSteps
        // The window size to come back to.
        property size origin
        property int step: 0
        readonly property var steps: [
            () => { origin = Qt.size(window.width, window.height); window.width = 390; window.height = 844 },
            () => window.tab = 1, () => window.tab = 2, () => window.tab = 3,
            () => { window.tab = 0; window.deckPair = 1 },
            () => { window.width = 844; window.height = 390 },
            () => window.tab = 1, () => window.tab = 2, () => window.tab = 3,
            () => { window.tab = 0; window.deckPair = 0; window.width = smokeSteps.origin.width; window.height = smokeSteps.origin.height }
        ]
        interval: 150
        repeat: true
        onTriggered: {
            if (step < steps.length)
                steps[step++]()
            else
                Qt.exit(0)
        }
    }
}
