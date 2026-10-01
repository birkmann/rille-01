pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Dialogs
import QtQuick.Layouts
import rille.ui

// Settings: sections on the left, their settings as cards on the right.
Popup {
    id: dialog
    modal: true
    focus: true
    width: Math.min(860, parent ? parent.width - 80 : 860)
    height: Math.min(600, parent ? parent.height - 80 : 600)
    anchors.centerIn: Overlay.overlay
    padding: 0

    property var s: ({})
    property var devices: []
    property var midi: ({ ports: [], connected: [], mappings: [] })
    property int page: 0
    readonly property var pages: [
        { name: "Audio", icon: "headphones" },
        { name: "Decks & Analysis", icon: "analyze" },
        { name: "Waveform", icon: "palette" },
        { name: "Library", icon: "library" },
        { name: "Controllers", icon: "midi" },
        { name: "Beatport", icon: "cloud" }
    ]
    // "12 tracks · 480 MB" in the Beatport cache.
    property string cacheText: ""

    function reload() {
        s = JSON.parse(AppController.settingsJson())
        devices = JSON.parse(AppController.audioDevicesJson())
        midi = JSON.parse(AppController.midiJson())
        cacheText = AppController.beatportCacheText()
    }
    function set(name, value) {
        AppController.setSetting(name, String(value))
        reload()
    }
    onOpened: reload()

    Connections {
        target: AppController
        function onMidiRevisionChanged() {
            if (dialog.opened)
                dialog.midi = JSON.parse(AppController.midiJson())
        }
    }

    Overlay.modal: Rectangle { color: "#b0000000" }
    background: Rectangle { color: Theme.panel; border.color: Theme.border; radius: 6 }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // --- Navigation ---------------------------------------------------
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 210
            color: Theme.bg
            radius: 6
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 4
                RowLayout {
                    Layout.bottomMargin: 14
                    spacing: 8
                    Icon { name: "sliders"; size: 18; color: Theme.textDim }
                    UiText { text: "Settings"; font.pixelSize: Theme.fontLarge + 2; font.bold: true }
                }
                Repeater {
                    model: dialog.pages
                    Rectangle {
                        id: nav
                        required property var modelData
                        required property int index
                        readonly property bool current: dialog.page === index
                        Layout.fillWidth: true
                        implicitHeight: 36
                        radius: 4
                        color: current ? Theme.selection : (navArea.containsMouse ? Theme.panelRaised : "transparent")
                        Rectangle { visible: nav.current; width: 3; height: parent.height - 12; y: 6; radius: 1; color: Theme.sync }
                        Row {
                            anchors.verticalCenter: parent.verticalCenter
                            x: 12
                            spacing: 10
                            Icon { anchors.verticalCenter: parent.verticalCenter; name: nav.modelData.icon; size: 16; color: nav.current ? Theme.sync : Theme.textDim }
                            UiText { text: nav.modelData.name; color: nav.current ? Theme.text : Theme.textDim; font.bold: nav.current }
                        }
                        MouseArea { id: navArea; anchors.fill: parent; hoverEnabled: true; onClicked: dialog.page = nav.index }
                    }
                }
                Item { Layout.fillHeight: true }
                Row {
                    spacing: 7
                    BrandMark { size: 14; anchors.verticalCenter: parent.verticalCenter }
                    UiText { text: "rille " + AppController.version(); color: Theme.textFaint; font.family: Theme.fontMono; font.pixelSize: Theme.fontSmall }
                }
            }
        }

        // --- Pages ------------------------------------------------------------
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: 52
                Layout.leftMargin: 20
                Layout.rightMargin: 10
                UiText { Layout.fillWidth: true; text: dialog.pages[dialog.page].name; font.pixelSize: Theme.fontLarge + 2; font.bold: true }
                Rectangle {
                    implicitWidth: 32
                    implicitHeight: 32
                    radius: 4
                    color: closeArea.containsMouse ? Theme.controlHover : "transparent"
                    Icon { anchors.centerIn: parent; name: "x"; size: 16; color: Theme.text }
                    MouseArea { id: closeArea; anchors.fill: parent; hoverEnabled: true; onClicked: dialog.close() }
                }
            }
            Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.border }

            StackLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: dialog.page

                // Audio
                Flickable {
                    clip: true
                    contentHeight: audioPage.implicitHeight + 40
                    boundsBehavior: Flickable.StopAtBounds
                    ColumnLayout {
                        id: audioPage
                        x: 20; y: 20
                        width: parent.width - 40
                        spacing: 14
                        SettingsSection {
                            title: "Output"
                            SettingRow {
                                label: "Output device"
                                hint: "Where the master (and headphone) mix plays."
                                StyledCombo {
                                    width: 300
                                    model: ["System default"].concat(dialog.devices)
                                    currentIndex: dialog.s.audio_device ? Math.max(0, dialog.devices.indexOf(dialog.s.audio_device) + 1) : 0
                                    onActivated: idx => dialog.set("audio_device", idx === 0 ? "" : dialog.devices[idx - 1])
                                }
                            }
                            SettingRow {
                                label: "Mixing"
                                hint: "External: every deck plays on its own pair of outputs into a hardware mixer such as the Allen & Heath Xone:96, which does the faders, EQ, crossfader and headphones. Automatic picks it when the output is such a mixer."
                                StyledCombo {
                                    id: mixingCombo
                                    readonly property var modes: ["auto", "internal", "external"]
                                    width: 180
                                    model: ["Automatic", "Internal mixer", "External mixer"]
                                    currentIndex: Math.max(0, mixingCombo.modes.indexOf(dialog.s.mixing))
                                    onActivated: idx => dialog.set("mixing", mixingCombo.modes[idx])
                                }
                            }
                            SettingRow {
                                visible: dialog.s.mixing !== "internal"
                                label: "Mixer channels"
                                hint: "The deck on each mixer channel, left to right: channel 1 gets outputs 1/2, channel 2 outputs 3/4, and so on. The Xone:96 defaults to C A B D."
                                StyledCombo {
                                    id: channelsCombo
                                    readonly property var orders: ["CABD", "ABCD"]
                                    width: 140
                                    model: channelsCombo.orders.map(o => o.split("").join(" "))
                                    currentIndex: Math.max(0, channelsCombo.orders.indexOf(dialog.s.mixer_channels))
                                    onActivated: idx => dialog.set("mixer_channels", channelsCombo.orders[idx])
                                }
                            }
                            SettingRow {
                                label: "Show the mixer"
                                hint: "The on-screen channels, EQ, faders and crossfader. Turn off when a controller has the knobs and faders, to give the decks the room."
                                ToggleSwitch { checked: dialog.s.show_mixer !== false; onToggled: dialog.set("show_mixer", checked) }
                            }
                            SettingRow {
                                label: "Buffer size"
                                hint: "Smaller is more direct, larger is safer against dropouts."
                                StyledCombo {
                                    id: bufferCombo
                                    readonly property var sizes: ["64", "128", "256", "512", "1024", "auto"]
                                    width: 140
                                    model: bufferCombo.sizes.map(v => v === "auto" ? "Automatic" : v + " frames")
                                    currentIndex: dialog.s.buffer_frames ? Math.max(0, bufferCombo.sizes.indexOf(String(dialog.s.buffer_frames))) : 5
                                    onActivated: idx => dialog.set("buffer_frames", bufferCombo.sizes[idx] === "auto" ? "" : bufferCombo.sizes[idx])
                                }
                            }
                            RowLayout {
                                spacing: 8
                                Rectangle { implicitWidth: 8; implicitHeight: 8; radius: 4; color: AppController.audioText.indexOf("No audio") < 0 ? Theme.play : Theme.danger }
                                UiText { Layout.fillWidth: true; text: AppController.audioText; color: Theme.textDim; elide: Text.ElideRight }
                            }
                            SettingRow {
                                label: "Level meter in the title bar"
                                hint: "The main output level, left and right, next to the clock, with the limiter's gain reduction."
                                ToggleSwitch { checked: !!dialog.s.header_meter; onToggled: dialog.set("header_meter", checked) }
                            }
                        }
                        SettingsSection {
                            title: "Headphones"
                            SettingRow {
                                label: "Split mono cue"
                                hint: "On 2-channel sound cards: headphones on the left channel, master on the right. With 4 or more channels, outputs 3/4 carry the headphone mix."
                                ToggleSwitch { checked: !!dialog.s.split_cue; onToggled: dialog.set("split_cue", checked) }
                            }
                        }
                    }
                }

                // Decks & analysis
                Flickable {
                    clip: true
                    contentHeight: decksPage.implicitHeight + 40
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: StyledScrollBar {}
                    ColumnLayout {
                        id: decksPage
                        x: 20; y: 20
                        width: parent.width - 40
                        spacing: 14
                        SettingsSection {
                            title: "Decks"
                            SettingRow {
                                label: "Deck layout"
                                hint: "4 decks: A and C on the left, B and D on the right, mixer channels in the order C A B D. Decks C and D keep playing when switched back to 2."
                                StyledCombo {
                                    width: 180
                                    model: ["2 decks (A B)", "4 decks (A B C D)"]
                                    currentIndex: dialog.s.deck_count === 4 ? 1 : 0
                                    onActivated: idx => dialog.set("deck_count", idx === 1 ? 4 : 2)
                                }
                            }
                            SettingRow {
                                label: "Remix decks"
                                hint: "Remix decks play loops and one-shots from four slots of sample cells, in time with the other decks (for a pad controller such as the Traktor Kontrol F1, which drives deck C by default). Their cells are kept when switched back."
                                Row {
                                    spacing: 4
                                    Repeater {
                                        model: ["A", "B", "C", "D"]
                                        DjButton {
                                            required property string modelData
                                            readonly property string decks: dialog.s.remix_decks || ""
                                            text: modelData
                                            implicitWidth: 34
                                            implicitHeight: 26
                                            lit: decks.indexOf(modelData) >= 0
                                            litColor: Theme.sync
                                            tip: "Deck " + modelData + (lit ? " is a remix deck" : " is a track deck")
                                            onClicked: dialog.set("remix_decks", lit ? decks.replace(modelData, "") : decks + modelData)
                                        }
                                    }
                                }
                            }
                            SettingRow {
                                label: "Tempo range"
                                hint: "Range of the tempo faders."
                                StyledCombo {
                                    id: rangeCombo
                                    readonly property var ranges: [0.02, 0.04, 0.06, 0.08, 0.10, 0.16, 0.25, 0.35, 0.5, 1.0]
                                    width: 140
                                    model: rangeCombo.ranges.map(r => "±" + Math.round(r * 100) + " %")
                                    currentIndex: Math.max(0, rangeCombo.ranges.indexOf(dialog.s.tempo_range))
                                    onActivated: idx => dialog.set("tempo_range", rangeCombo.ranges[idx])
                                }
                            }
                            SettingRow {
                                label: "Key notation"
                                StyledCombo {
                                    id: keyCombo
                                    readonly property var values: ["camelot", "open_key", "musical"]
                                    width: 180
                                    model: ["Camelot (8A)", "Open Key (1m)", "Musical (Am)"]
                                    currentIndex: Math.max(0, keyCombo.values.indexOf(dialog.s.key_notation))
                                    onActivated: idx => dialog.set("key_notation", keyCombo.values[idx])
                                }
                            }
                            SettingRow {
                                label: "Auto gain"
                                hint: "Levels every track to the same loudness when it is loaded."
                                ToggleSwitch { checked: !!dialog.s.auto_gain; onToggled: dialog.set("auto_gain", checked) }
                            }
                            SettingRow {
                                label: "Target loudness"
                                StyledSpin {
                                    from: -20; to: -4
                                    suffix: "LUFS"
                                    value: Math.round(dialog.s.target_lufs || -10)
                                    onValueModified: dialog.set("target_lufs", value)
                                }
                            }
                        }
                        SettingsSection {
                            title: "Analysis"
                            SettingRow {
                                label: "Analyze in the background"
                                hint: "New and changed tracks are analyzed automatically."
                                ToggleSwitch { checked: !!dialog.s.background_analysis; onToggled: dialog.set("background_analysis", checked) }
                            }
                            SettingRow {
                                label: "BPM range"
                                hint: "Detected tempos are folded into this range (half/double time). Applies to tracks analyzed afterwards."
                                RowLayout {
                                    spacing: 6
                                    StyledSpin { from: 40; to: 200; value: Math.round(dialog.s.bpm_min || 88); onValueModified: dialog.set("bpm_min", value) }
                                    UiText { text: "to"; color: Theme.textDim }
                                    StyledSpin { from: 60; to: 300; value: Math.round(dialog.s.bpm_max || 175); onValueModified: dialog.set("bpm_max", value) }
                                }
                            }
                        }
                    }
                }

                // Waveform
                Flickable {
                    clip: true
                    contentHeight: wavePage.implicitHeight + 40
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: StyledScrollBar {}
                    ColumnLayout {
                        id: wavePage
                        x: 20; y: 20
                        width: parent.width - 40
                        spacing: 14
                        SettingsSection {
                            title: "Look"
                            SettingRow {
                                label: "Style"
                                hint: styleCombo.hints[styleCombo.currentIndex]
                                StyledCombo {
                                    id: styleCombo
                                    readonly property var values: ["spectrum", "three_band", "rgb", "mono"]
                                    readonly property var hints: [
                                        "Bass red, mids green, highs blue, mixed like light, with a bright core for hats and claps.",
                                        "Bands stacked on top of each other: lows blue, mids amber, highs white, as on club media players.",
                                        "One saturated color per column from the frequency mix: red kicks, green vocals, blue hats.",
                                        "Classic single blue, turning white where the highs dominate."
                                    ]
                                    width: 180
                                    model: ["Spectrum", "3-Band", "RGB", "Mono blue"]
                                    currentIndex: Math.max(0, styleCombo.values.indexOf(dialog.s.waveform_style))
                                    onActivated: idx => dialog.set("waveform_style", styleCombo.values[idx])
                                }
                            }
                            SettingRow {
                                label: "Shape"
                                hint: "Mirrored around the centre line, or growing up from the bottom edge."
                                StyledCombo {
                                    width: 180
                                    model: ["Mirrored", "From the bottom"]
                                    currentIndex: dialog.s.waveform_bottom ? 1 : 0
                                    onActivated: idx => dialog.set("waveform_bottom", idx === 1)
                                }
                            }
                            SettingRow {
                                label: "Height"
                                hint: "Vertical scale of the waveforms. Larger shows quiet parts better; loud parts reach the edge."
                                StyledCombo {
                                    id: heightCombo
                                    readonly property var values: [0.5, 0.75, 1.0, 1.25, 1.5, 2.0]
                                    width: 140
                                    model: heightCombo.values.map(v => Math.round(v * 100) + " %")
                                    currentIndex: Math.max(0, heightCombo.values.indexOf(dialog.s.waveform_height))
                                    onActivated: idx => dialog.set("waveform_height", heightCombo.values[idx])
                                }
                            }
                        }
                        SettingsSection {
                            title: "Mixer feedback"
                            SettingRow {
                                label: "Show EQ, filter and gain"
                                hint: "The scrolling waveform follows the channel: a killed EQ band disappears (its outline stays in grey), the filter removes lows or highs, and the GAIN knob makes the waveform larger or smaller."
                                ToggleSwitch { checked: !!dialog.s.waveform_mixer; onToggled: dialog.set("waveform_mixer", checked) }
                            }
                            SettingRow {
                                label: "Dim with fader"
                                hint: "The scrolling waveform fades with the channel fader and crossfader, so you see which decks are audible. It stays visible for cueing."
                                ToggleSwitch { checked: !!dialog.s.waveform_fader_dim; onToggled: dialog.set("waveform_fader_dim", checked) }
                            }
                        }
                    }
                }

                // Library
                Flickable {
                    clip: true
                    contentHeight: libraryPage.implicitHeight + 40
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: StyledScrollBar {}
                    ColumnLayout {
                        id: libraryPage
                        x: 20; y: 20
                        width: parent.width - 40
                        spacing: 14
                        SettingsSection {
                            title: "Music folders"
                            UiText {
                                visible: (dialog.s.library_roots || []).length === 0
                                text: "No music folders yet. Add the folders that hold your music; they are scanned for new files on every start."
                                color: Theme.textDim
                                wrapMode: Text.WordWrap
                                Layout.fillWidth: true
                            }
                            Repeater {
                                model: dialog.s.library_roots || []
                                Rectangle {
                                    id: folderRow
                                    required property string modelData
                                    Layout.fillWidth: true
                                    implicitHeight: 36
                                    radius: 4
                                    color: Theme.bg
                                    border.color: Theme.panelEdge
                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: 10
                                        anchors.rightMargin: 4
                                        spacing: 8
                                        Icon { name: "folder"; size: 16; color: Theme.sync }
                                        UiText { Layout.fillWidth: true; text: folderRow.modelData; elide: Text.ElideMiddle }
                                        DjButton {
                                            flat: true
                                            icon: "trash"
                                            implicitWidth: 30
                                            onClicked: {
                                                AppController.removeMusicFolder(folderRow.modelData)
                                                dialog.reload()
                                            }
                                        }
                                    }
                                }
                            }
                            RowLayout {
                                spacing: 6
                                DjButton { icon: "plus"; text: "ADD FOLDER"; onClicked: folderDialog.open() }
                                DjButton { icon: "refresh"; text: "RESCAN"; onClicked: AppController.rescan() }
                            }
                        }
                        SettingsSection {
                            title: "Import"
                            SettingRow {
                                label: "Traktor collection"
                                hint: "Brings over beatgrids you corrected, cue points, loops and ratings from a collection.nml. Imported grids are kept when tracks are re-analyzed."
                                DjButton { icon: "import"; text: "IMPORT…"; onClicked: nmlDialog.open() }
                            }
                        }
                    }
                }

                // Controllers
                Flickable {
                    clip: true
                    contentHeight: controllerPage.implicitHeight + 40
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: StyledScrollBar {}
                    ColumnLayout {
                        id: controllerPage
                        x: 20; y: 20
                        width: parent.width - 40
                        spacing: 14
                        SettingsSection {
                            title: "Connected controllers"
                            UiText {
                                visible: dialog.midi.ports.length === 0
                                Layout.fillWidth: true
                                text: "No controllers found. Plug one in; it appears here within a few seconds."
                                color: Theme.textDim
                                wrapMode: Text.WordWrap
                            }
                            Repeater {
                                model: dialog.midi.ports
                                Rectangle {
                                    id: portRow
                                    required property string modelData
                                    readonly property string current: {
                                        for (var i = 0; i < dialog.midi.connected.length; i++)
                                            if (dialog.midi.connected[i].port === portRow.modelData)
                                                return dialog.midi.connected[i].mapping || ""
                                        return ""
                                    }
                                    Layout.fillWidth: true
                                    implicitHeight: 42
                                    radius: 4
                                    color: Theme.bg
                                    border.color: Theme.panelEdge
                                    RowLayout {
                                        anchors.fill: parent
                                        anchors.leftMargin: 10
                                        anchors.rightMargin: 6
                                        spacing: 8
                                        Rectangle { implicitWidth: 8; implicitHeight: 8; radius: 4; color: portRow.current.length ? Theme.play : Theme.textFaint }
                                        UiText { Layout.fillWidth: true; text: portRow.modelData; elide: Text.ElideRight; font.bold: true }
                                        StyledCombo {
                                            Layout.preferredWidth: 280
                                            model: ["No mapping"].concat(dialog.midi.mappings)
                                            currentIndex: portRow.current.length ? Math.max(0, dialog.midi.mappings.indexOf(portRow.current) + 1) : 0
                                            onActivated: idx => AppController.setPortMapping(portRow.modelData, idx === 0 ? "" : dialog.midi.mappings[idx - 1])
                                        }
                                    }
                                }
                            }
                        }
                        SettingsSection {
                            title: "MIDI learn"
                            SettingRow {
                                label: "Learn a control"
                                hint: "Click a control on screen, then move a knob or press a button on the controller. Learned mappings are saved to ~/.config/rille/mappings."
                                DjButton {
                                    icon: "midi"
                                    text: AppController.learning ? "STOP" : "START"
                                    lit: AppController.learning
                                    litColor: Theme.warn
                                    onClicked: AppController.setLearn(!AppController.learning)
                                }
                            }
                            UiText {
                                visible: AppController.learnText.length > 0
                                Layout.fillWidth: true
                                text: AppController.learnText
                                color: Theme.warn
                                elide: Text.ElideRight
                            }
                        }
                    }
                }

                // Beatport
                Flickable {
                    clip: true
                    contentHeight: beatportPage.implicitHeight + 40
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: StyledScrollBar {}
                    ColumnLayout {
                        id: beatportPage
                        readonly property bool signedIn: AppController.beatportAccount.length > 0
                        x: 20; y: 20
                        width: parent.width - 40
                        spacing: 14
                        SettingsSection {
                            title: "Account"
                            UiText {
                                Layout.fillWidth: true
                                text: "Stream tracks from the Beatport catalog and your Beatport playlists onto the decks. Needs a Beatport streaming subscription. rille keeps the sign-in, never your password."
                                color: Theme.textDim
                                wrapMode: Text.WordWrap
                            }
                            RowLayout {
                                visible: beatportPage.signedIn
                                spacing: 8
                                Rectangle { implicitWidth: 8; implicitHeight: 8; radius: 4; color: Theme.play }
                                UiText { Layout.fillWidth: true; text: "Signed in as " + AppController.beatportAccount; font.bold: true; elide: Text.ElideRight }
                                DjButton { icon: "x"; text: "SIGN OUT"; onClicked: AppController.beatportLogout() }
                            }
                            RowLayout {
                                visible: !beatportPage.signedIn
                                spacing: 6
                                TextField {
                                    id: bpUser
                                    Layout.preferredWidth: 220
                                    implicitHeight: 28
                                    placeholderText: "Email or username"
                                    placeholderTextColor: Theme.textFaint
                                    color: Theme.text
                                    font.pixelSize: Theme.fontNormal
                                    background: Rectangle { color: Theme.bg; radius: Theme.radius; border.color: bpUser.activeFocus ? Theme.sync : Theme.border }
                                    onAccepted: bpPassword.forceActiveFocus()
                                }
                                TextField {
                                    id: bpPassword
                                    Layout.preferredWidth: 180
                                    implicitHeight: 28
                                    placeholderText: "Password"
                                    placeholderTextColor: Theme.textFaint
                                    // The eye shows the typed password to check it.
                                    property bool revealed: false
                                    echoMode: revealed ? TextInput.Normal : TextInput.Password
                                    rightPadding: 30
                                    color: Theme.text
                                    font.pixelSize: Theme.fontNormal
                                    background: Rectangle { color: Theme.bg; radius: Theme.radius; border.color: bpPassword.activeFocus ? Theme.sync : Theme.border }
                                    onAccepted: signIn.clicked()
                                    Icon {
                                        anchors.right: parent.right
                                        anchors.rightMargin: 8
                                        anchors.verticalCenter: parent.verticalCenter
                                        name: bpPassword.revealed ? "eye-off" : "eye"
                                        size: 15
                                        color: eyeArea.containsMouse || bpPassword.revealed ? Theme.text : Theme.textDim
                                        MouseArea {
                                            id: eyeArea
                                            anchors.fill: parent
                                            anchors.margins: -6
                                            hoverEnabled: true
                                            cursorShape: Qt.PointingHandCursor
                                            onClicked: bpPassword.revealed = !bpPassword.revealed
                                        }
                                        Tip { text: bpPassword.revealed ? "Hide the password" : "Show the password"; visible: eyeArea.containsMouse }
                                    }
                                }
                                DjButton {
                                    id: signIn
                                    icon: "check"
                                    text: "SIGN IN"
                                    enabled: bpUser.text.length > 0 && bpPassword.text.length > 0
                                    onClicked: {
                                        AppController.beatportLogin(bpUser.text, bpPassword.text)
                                        bpPassword.text = ""
                                    }
                                }
                            }
                            UiText {
                                visible: AppController.status.indexOf("Beatport") >= 0
                                Layout.fillWidth: true
                                text: AppController.status
                                color: Theme.textDim
                                elide: Text.ElideRight
                            }
                        }
                        SettingsSection {
                            title: "Streaming"
                            SettingRow {
                                label: "Quality"
                                hint: "Lossless FLAC and AAC 256 need a Professional plan, AAC 128 an Advanced one. Applies to tracks downloaded afterwards."
                                StyledCombo {
                                    id: qualityCombo
                                    readonly property var values: ["lossless", "high", "medium"]
                                    width: 200
                                    model: ["Lossless (FLAC)", "AAC 256 kbps", "AAC 128 kbps"]
                                    currentIndex: Math.max(0, qualityCombo.values.indexOf(dialog.s.beatport_quality))
                                    onActivated: idx => dialog.set("beatport_quality", qualityCombo.values[idx])
                                }
                            }
                        }
                        SettingsSection {
                            title: "Offline storage"
                            SettingRow {
                                label: "Storage limit"
                                hint: "Every streamed track stays on disk, so it loads at once next time and plays without a connection. When the limit is reached, the least recently played make room, never the ones you downloaded for offline use. Cues, beatgrids and analysis are kept either way."
                                StyledSpin {
                                    from: 1; to: 500
                                    suffix: "GB"
                                    value: Math.max(1, Math.round((dialog.s.beatport_cache_mb || 20480) / 1024))
                                    onValueModified: dialog.set("beatport_cache_mb", value * 1024)
                                }
                            }
                            SettingRow {
                                label: "On this computer"
                                hint: dialog.cacheText
                                Row {
                                    spacing: 6
                                    DjButton {
                                        icon: "trash"
                                        text: "CLEAR STREAMED"
                                        tip: "Delete the files of streamed tracks you did not download for offline use (not those on a deck); they download again when loaded"
                                        onClicked: {
                                            AppController.clearBeatportCache(false)
                                            dialog.cacheText = AppController.beatportCacheText()
                                        }
                                    }
                                    DjButton {
                                        icon: "x"
                                        text: "REMOVE ALL"
                                        tip: "Delete every Beatport file, also the offline downloads (not those on a deck)"
                                        onClicked: {
                                            AppController.clearBeatportCache(true)
                                            dialog.cacheText = AppController.beatportCacheText()
                                        }
                                    }
                                }
                            }
                            UiText {
                                Layout.fillWidth: true
                                text: "Download a playlist for offline use: right-click it in the browser, or DOWNLOAD ALL above any Beatport list. A green check marks downloaded tracks."
                                color: Theme.textDim
                                wrapMode: Text.WordWrap
                            }
                        }
                    }
                }
            }
        }
    }

    FolderDialog {
        id: folderDialog
        title: "Add music folder"
        onAccepted: {
            AppController.addMusicFolder(selectedFolder)
            dialog.reload()
        }
    }
    FileDialog {
        id: nmlDialog
        title: "Traktor collection.nml"
        nameFilters: ["Traktor collection (*.nml)"]
        onAccepted: AppController.importNml(selectedFile)
    }
}
