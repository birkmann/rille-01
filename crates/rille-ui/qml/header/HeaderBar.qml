pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// The window's title bar (the window has no system frame): drag to move,
// double-click to maximize, window buttons on the right.
Rectangle {
    id: header
    property bool maximized: false
    /// Mobile layout: lights and level only, the buttons in a menu.
    property bool compact: false
    // The main level, if turned on and there is room for it.
    readonly property bool showMeter: AppController.headerMeter && !compact && width >= 1100
    signal settingsRequested()
    signal aboutRequested()
    signal fullscreenRequested()
    signal moveRequested()
    signal minimizeRequested()
    signal maximizeRequested()
    signal closeRequested()
    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    // Empty parts of the bar move the window; buttons take their own clicks.
    DragHandler {
        target: null
        onActiveChanged: {
            if (active)
                header.moveRequested()
        }
    }
    TapHandler {
        onDoubleTapped: header.maximizeRequested()
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 10
        anchors.rightMargin: header.compact ? 4 : 8
        spacing: header.compact ? 10 : 12

        // The lockup; opens About.
        Item {
            Layout.alignment: Qt.AlignVCenter
            Layout.rightMargin: 4
            implicitWidth: lockup.implicitWidth
            implicitHeight: lockup.implicitHeight
            opacity: logoArea.containsMouse ? 0.8 : 1
            Row {
                id: lockup
                spacing: 7
                BrandMark { size: 20; anchors.verticalCenter: parent.verticalCenter }
                Wordmark { visible: !header.compact; size: 19; anchors.verticalCenter: parent.verticalCenter; anchors.verticalCenterOffset: -1 }
            }
            MouseArea {
                id: logoArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: header.aboutRequested()
            }
            Tip { text: "About rille"; below: true; visible: logoArea.containsMouse }
        }
        Rectangle { visible: !header.compact; Layout.preferredWidth: 1; Layout.preferredHeight: 18; color: Theme.border }
        // Audio and MIDI status lights.
        Row {
            spacing: 8
            Layout.alignment: Qt.AlignVCenter
            Repeater {
                model: [
                    { name: "AUDIO", ok: AppController.audioText.indexOf("No audio") < 0 && AppController.audioText.indexOf("none") !== 0, tip: "Audio output: " + AppController.audioText },
                    { name: "MIDI", ok: AppController.midiRevision >= 0 && AppController.midiConnected, tip: AppController.midiConnected ? "A controller is connected" : "No controller connected" }
                ]
                Row {
                    id: led
                    required property var modelData
                    spacing: 4
                    HoverHandler { id: ledHover }
                    Tip { text: led.modelData.tip; below: true; visible: ledHover.hovered }
                    Rectangle { width: 7; height: 7; radius: 4; anchors.verticalCenter: parent.verticalCenter; color: led.modelData.ok ? Theme.play : Theme.textFaint }
                    UiText { visible: !header.compact; text: led.modelData.name; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true }
                }
            }
        }
        // CPU load.
        Row {
            spacing: 4
            HoverHandler { id: cpuHover }
            Tip { text: "Audio CPU load: red means dropouts are close; a larger buffer (Settings) helps. Dropouts so far are counted next to it."; below: true; visible: cpuHover.hovered }
            Layout.alignment: Qt.AlignVCenter
            UiText { visible: !header.compact; text: "CPU"; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true; anchors.verticalCenter: parent.verticalCenter }
            Rectangle {
                width: header.compact ? 28 : 46; height: 6; radius: 2; color: Theme.control
                anchors.verticalCenter: parent.verticalCenter
                Rectangle {
                    width: parent.width * Math.min(1, AppController.cpuLoad)
                    height: parent.height; radius: 2
                    color: AppController.cpuLoad > 0.8 ? Theme.danger : Theme.sync
                }
            }
            UiText {
                visible: AppController.xruns > 0
                text: AppController.xruns + (header.compact ? "" : (AppController.xruns === 1 ? " DROPOUT" : " DROPOUTS"))
                color: Theme.danger; font.pixelSize: Theme.fontTiny; font.bold: true
                anchors.verticalCenter: parent.verticalCenter
            }
        }
        // Laptop battery, only while unplugged: charge and runtime left.
        Row {
            id: battery
            readonly property int percent: AppController.batteryPercent
            readonly property int minutes: AppController.batteryMinutes
            readonly property bool critical: percent >= 0 && percent <= 10
            readonly property color tint: critical ? Theme.danger : (percent <= 20 ? Theme.warn : Theme.text)
            visible: AppController.onBattery
            spacing: 4
            Layout.alignment: Qt.AlignVCenter
            // Pulses when nearly flat.
            SequentialAnimation on opacity {
                running: battery.visible && battery.critical
                loops: Animation.Infinite
                onRunningChanged: if (!running) battery.opacity = 1
                NumberAnimation { to: 0.35; duration: 600 }
                NumberAnimation { to: 1; duration: 600 }
            }
            Row {
                anchors.verticalCenter: parent.verticalCenter
                Rectangle {
                    width: 24; height: 12; radius: 2
                    color: "transparent"
                    border.color: battery.tint; border.width: 1.5
                    Rectangle {
                        x: 2.5; y: 2.5; radius: 1
                        height: parent.height - 5
                        width: (parent.width - 5) * Math.max(0.06, battery.percent / 100)
                        color: battery.tint
                    }
                }
                Rectangle { width: 2; height: 5; radius: 1; color: battery.tint; anchors.verticalCenter: parent.verticalCenter }
            }
            UiText {
                anchors.verticalCenter: parent.verticalCenter
                text: battery.percent + "%" + (battery.minutes >= 0 && !header.compact ? "  " + Math.floor(battery.minutes / 60) + ":" + String(battery.minutes % 60).padStart(2, "0") : "")
                color: battery.tint
                font.pixelSize: Theme.fontSmall
                font.bold: true
            }
        }
        // Main level, horizontal (off by default: Settings → Audio).
        UiText { visible: header.showMeter; text: "MAIN"; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true }
        Column {
            visible: header.showMeter
            spacing: 2
            HoverHandler { id: mainHover }
            Tip { text: "Main output level, left and right"; below: true; visible: mainHover.hovered }
            Layout.alignment: Qt.AlignVCenter
            Repeater {
                model: [AppController.masterPeakL, AppController.masterPeakR]
                Rectangle {
                    required property real modelData
                    readonly property real db: modelData > 0 ? 20 * Math.log(modelData) / Math.LN10 : -60
                    width: 160; height: 4; radius: 1; color: Theme.control
                    Rectangle {
                        height: parent.height; radius: 1
                        width: parent.width * Math.max(0, Math.min(1, (parent.db + 48) / 51))
                        color: parent.db > -1 ? Theme.danger : (parent.db > -6 ? Theme.warn : Theme.sync)
                    }
                }
            }
        }
        UiText {
            visible: header.showMeter && AppController.limiterReduction > 0.5
            text: "LIM −" + AppController.limiterReduction.toFixed(1)
            color: Theme.warn
            font.pixelSize: Theme.fontTiny
        }
        UiText {
            id: clock
            visible: !header.compact || header.width > 520
            color: Theme.text
            font.family: Theme.fontMono
            font.pixelSize: Theme.fontNormal
            font.weight: Font.Medium
            Timer {
                interval: 1000; running: true; repeat: true; triggeredOnStart: true
                onTriggered: clock.text = Qt.formatTime(new Date(), "hh:mm")
            }
        }
        UiText {
            Layout.fillWidth: true
            text: AppController.learning ? AppController.learnText : (header.compact ? "" : AppController.audioText)
            color: AppController.learning ? Theme.warn : Theme.textFaint
            font.pixelSize: Theme.fontSmall
            elide: Text.ElideRight
            horizontalAlignment: Text.AlignRight
        }
        DjButton {
            icon: "dot"
            text: AppController.recording ? (header.compact ? "" : "REC ") + AppController.recordingTime : (header.compact ? "" : "REC")
            target: "global.record"
            lit: AppController.recording
            litColor: Theme.danger
            tip: AppController.recording
                 ? "Recording the main mix to " + AppController.recordingFile + ". Click to stop."
                 : "Record the main mix to a WAV file (with a cue sheet of the tracks played) in your music folder's \"rille recordings\""
            tipBelow: true
        }
        DjButton {
            visible: !header.compact
            icon: "drums"
            text: "DRUMS"
            lit: AppController.drumsVisible
            litColor: Theme.sync
            tip: AppController.drumsVisible ? "Hide the drum machine (it keeps playing)" : "Show the drum machine"
            tipBelow: true
            onClicked: AppController.setSetting("drums_visible", AppController.drumsVisible ? "false" : "true")
            // Playing while hidden.
            Rectangle {
                visible: AppController.drumsPlaying && !AppController.drumsVisible
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 3
                width: 6
                height: 6
                radius: 3
                color: Theme.play
                opacity: ((AppController.clockBeat % 1) + 1) % 1 < 0.5 ? 1 : 0.3
            }
        }
        DjButton {
            visible: !header.compact
            icon: "midi"
            text: "LEARN"
            lit: AppController.learning
            litColor: Theme.warn
            tip: "MIDI learn: click a control on screen, then move a knob or press a button on the controller"
            tipBelow: true
            onClicked: AppController.setLearn(!AppController.learning)
        }
        DjButton {
            visible: !header.compact
            icon: "sliders"
            text: "SETTINGS"
            onClicked: header.settingsRequested()
        }
        DjButton {
            visible: !header.compact
            icon: "maximize"
            implicitWidth: 30
            tip: "Fullscreen"
            tipBelow: true
            onClicked: header.fullscreenRequested()
        }
        // Compact: everything else in a menu.
        DjButton {
            id: moreButton
            visible: header.compact
            icon: "more"
            implicitWidth: 40
            implicitHeight: 34
            lit: AppController.learning
            litColor: Theme.warn
            onClicked: moreMenu.popup(moreButton, 0, moreButton.height + 4)
        }
        Rectangle { visible: !header.compact; Layout.preferredWidth: 1; Layout.preferredHeight: 18; color: Theme.border }
        DjButton { visible: !header.compact; flat: true; icon: "minus"; implicitWidth: 32; tip: "Minimize"; tipBelow: true; onClicked: header.minimizeRequested() }
        DjButton { visible: !header.compact; flat: true; icon: header.maximized ? "restore" : "square"; implicitWidth: 32; tip: header.maximized ? "Restore" : "Maximize"; tipBelow: true; onClicked: header.maximizeRequested() }
        Rectangle {
            implicitWidth: header.compact ? 40 : 34
            implicitHeight: header.compact ? 34 : 28
            radius: Theme.radius
            color: closeArea.containsMouse ? Theme.danger : "transparent"
            Icon { anchors.centerIn: parent; name: "x"; size: 15; color: closeArea.containsMouse ? Theme.brandPaper : Theme.text }
            MouseArea {
                id: closeArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: header.closeRequested()
            }
        }
    }

    StyledMenu {
        id: moreMenu
        StyledMenuItem { iconName: "sliders"; text: "Settings"; onTriggered: header.settingsRequested() }
        StyledMenuItem { iconName: "midi"; text: "MIDI learn"; marked: AppController.learning; onTriggered: AppController.setLearn(!AppController.learning) }
        StyledMenuSeparator {}
        StyledMenuItem { iconName: "maximize"; text: "Fullscreen"; onTriggered: header.fullscreenRequested() }
        StyledMenuItem { iconName: "minus"; text: "Minimize"; onTriggered: header.minimizeRequested() }
        StyledMenuItem { iconName: header.maximized ? "restore" : "square"; text: header.maximized ? "Restore" : "Maximize"; onTriggered: header.maximizeRequested() }
        StyledMenuSeparator {}
        StyledMenuItem { iconName: "music"; text: "About rille"; onTriggered: header.aboutRequested() }
    }
}
