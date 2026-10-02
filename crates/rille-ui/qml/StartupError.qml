pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import rille.ui

// Shown instead of the main window when the app cannot start: the error,
// plus retrying or starting over with an empty library.
ApplicationWindow {
    id: window
    width: 560
    height: content.implicitHeight + 48
    visible: true
    title: "rille"
    color: Theme.bg
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fontNormal

    property bool confirming: false
    property string resetError: ""

    ColumnLayout {
        id: content
        anchors.fill: parent
        anchors.margins: 24
        spacing: 14

        RowLayout {
            spacing: 12
            BrandMark { size: 32 }
            UiText {
                text: "rille could not start"
                color: Theme.text
                font.pixelSize: Theme.fontLarge
                font.weight: Font.DemiBold
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: errorText.implicitHeight + 20
            color: Theme.panel
            border.color: Theme.border
            radius: Theme.radius
            TextEdit {
                id: errorText
                anchors.fill: parent
                anchors.margins: 10
                text: Startup.error
                readOnly: true
                selectByMouse: true
                wrapMode: Text.Wrap
                color: Theme.danger
                font.family: Theme.fontMono
                font.pixelSize: Theme.fontSmall
            }
        }

        UiText {
            Layout.fillWidth: true
            visible: window.confirming
            wrapMode: Text.WordWrap
            lineHeight: 1.3
            color: Theme.textDim
            text: "Resetting moves the library database aside as a backup and starts with an empty library. "
                + "Playlists, cue points and analysis start over; your music files are not touched.\n\n"
                + Startup.library
        }

        UiText {
            Layout.fillWidth: true
            visible: window.resetError.length > 0
            wrapMode: Text.WordWrap
            color: Theme.danger
            text: window.resetError
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: 6
            spacing: 8

            DjButton {
                visible: Startup.canReset && !window.confirming
                text: "Reset library…"
                onClicked: window.confirming = true
            }
            DjButton {
                visible: window.confirming
                text: "Reset and start"
                lit: true
                litColor: Theme.danger
                onClicked: {
                    window.resetError = Startup.resetLibrary()
                    if (window.resetError.length === 0)
                        Qt.quit()
                }
            }
            DjButton {
                visible: window.confirming
                text: "Cancel"
                onClicked: window.confirming = false
            }
            Item { Layout.fillWidth: true }
            DjButton {
                text: "Quit"
                onClicked: Qt.quit()
            }
            DjButton {
                text: "Retry"
                lit: !window.confirming
                onClicked: {
                    Startup.retry()
                    Qt.quit()
                }
            }
        }
    }
}
