import QtQuick
import rille.ui

// Instrument selector: click selects (the sequencer row and knobs follow),
// double-click or the ▸ on hover plays it, middle-click mutes, right-click
// opens its menu. A dot shows that it has steps; it flashes on every hit.
// Drop a track or file on it to make it the instrument's sound.
Rectangle {
    id: tab
    property int inst: 0
    property string name: ""
    property string fullName: ""
    property color tint: Theme.sync
    property bool selected: false
    property bool used: false
    property bool muted: false
    property bool loaded: true
    /// Hit counter from the engine; a change is a hit.
    property string hits: ""

    signal menuRequested()

    onHitsChanged: if (!muted) flash.restart()

    radius: 3
    color: selected ? Qt.darker(tint, 2.8) : (area.containsMouse || drop.containsDrag ? Theme.controlHover : Theme.control)
    border.color: drop.containsDrag ? Theme.sync : (selected ? tint : Theme.border)

    Rectangle {
        id: glow
        anchors.fill: parent
        radius: 3
        color: tab.tint
        opacity: 0
        NumberAnimation on opacity {
            id: flash
            running: false
            from: 0.5
            to: 0
            duration: 160
        }
    }
    // Selected: a bar along the bottom.
    Rectangle {
        visible: tab.selected
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 1
        height: 2
        color: tab.tint
    }
    Row {
        anchors.centerIn: parent
        spacing: 5
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 6
            height: 6
            radius: 3
            color: tab.used ? tab.tint : Theme.meterOff
        }
        UiText {
            anchors.verticalCenter: parent.verticalCenter
            text: tab.name
            font.pixelSize: Theme.fontSmall
            font.bold: true
            font.strikeout: tab.muted
            font.italic: !tab.loaded
            color: tab.muted || !tab.loaded ? Theme.textFaint : (tab.selected ? Theme.text : Theme.textDim)
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
        onPressed: mouse => {
            if (mouse.button === Qt.RightButton)
                tab.menuRequested()
            else if (mouse.button === Qt.MiddleButton)
                AppController.press("drum.inst_mute." + (tab.inst + 1), true)
            else
                AppController.press("drum.inst." + (tab.inst + 1), true)
        }
        onReleased: mouse => {
            if (mouse.button === Qt.MiddleButton)
                AppController.press("drum.inst_mute." + (tab.inst + 1), false)
            else if (mouse.button === Qt.LeftButton)
                AppController.press("drum.inst." + (tab.inst + 1), false)
        }
        // Touch: press and hold for the menu.
        onPressAndHold: mouse => {
            if (Theme.mobile && mouse.button === Qt.LeftButton)
                tab.menuRequested()
        }
        onDoubleClicked: mouse => {
            if (mouse.button === Qt.LeftButton) {
                AppController.press("drum.trigger." + (tab.inst + 1), true)
                AppController.press("drum.trigger." + (tab.inst + 1), false)
            }
        }
    }
    // Play it (hover).
    Rectangle {
        visible: area.containsMouse || playArea.containsMouse
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.rightMargin: 2
        width: 18
        height: parent.height - 4
        radius: 2
        color: playArea.pressed ? tab.tint : (playArea.containsMouse ? Theme.controlHover : "transparent")
        Icon {
            anchors.centerIn: parent
            name: "play"
            size: 10
            color: playArea.pressed ? Theme.textOnLit : Theme.textDim
        }
        MouseArea {
            id: playArea
            anchors.fill: parent
            hoverEnabled: true
            onPressed: AppController.press("drum.trigger." + (tab.inst + 1), true)
            onReleased: AppController.press("drum.trigger." + (tab.inst + 1), false)
        }
    }
    Tip {
        visible: area.containsMouse && !area.pressed
        text: tab.fullName + (tab.loaded ? "" : " (no sound in this kit)")
            + "\nClick: select · double-click or ▸: play · middle-click: mute · right-click: sound and steps"
            + "\nDrop a track or file here to make it this instrument's sound"
    }
    DropArea {
        id: drop
        anchors.fill: parent
        keys: ["application/x-rille-track", "application/x-rille-path", "text/uri-list"]
        onDropped: d => {
            if (d.hasUrls && d.urls.length > 0)
                AppController.drumLoadUrl(tab.inst, d.urls[0])
            else if (d.getDataAsString("application/x-rille-track").length)
                AppController.drumLoadTrack(tab.inst, Number(d.getDataAsString("application/x-rille-track")))
            else if (d.getDataAsString("application/x-rille-path").length)
                AppController.drumLoadPath(tab.inst, Number(d.getDataAsString("application/x-rille-path")))
            d.acceptProposedAction()
        }
    }
}
