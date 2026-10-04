pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import rille.ui

// About rille: the lockup, version, license and credits.
Popup {
    id: about
    modal: true
    focus: true
    width: Math.min(440, parent ? parent.width - 16 : 440)
    anchors.centerIn: Overlay.overlay
    padding: 0

    Overlay.modal: Rectangle { color: "#b0000000" }
    background: Rectangle { color: Theme.brandInk; border.color: Theme.border; radius: Theme.radiusLarge }

    ColumnLayout {
        width: parent.width
        spacing: 0

        // Close button, top right.
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 40
            Rectangle {
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 8
                width: 32
                height: 32
                radius: 4
                color: closeArea.containsMouse ? Theme.controlHover : "transparent"
                Icon { anchors.centerIn: parent; name: "x"; size: 16; color: Theme.textDim }
                MouseArea { id: closeArea; anchors.fill: parent; hoverEnabled: true; onClicked: about.close() }
            }
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: 18
            BrandMark { size: 64 }
            Wordmark { size: 64 }
        }
        UiText {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 14
            text: "dj software for linux"
            color: Theme.textDim
            font.family: Theme.fontMono
            font.pixelSize: Theme.fontNormal
        }
        UiText {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 4
            text: "version " + AppController.version()
            color: Theme.textFaint
            font.family: Theme.fontMono
            font.pixelSize: Theme.fontSmall
        }

        Rectangle { Layout.fillWidth: true; Layout.topMargin: 28; Layout.preferredHeight: 1; color: Theme.border }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.margins: 24
            spacing: 10
            UiText {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Free software under the GNU General Public License, version 3 or later. Comes with no warranty."
                color: Theme.textDim
                font.pixelSize: Theme.fontSmall
                lineHeight: 1.3
            }
            UiText {
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                text: "Set in Geist and Geist Mono (SIL Open Font License). Time stretching by Signalsmith Stretch. Built with Rust and Qt."
                color: Theme.textFaint
                font.pixelSize: Theme.fontSmall
                lineHeight: 1.3
            }
            DjButton {
                visible: AppController.repository() !== ""
                Layout.topMargin: 6
                icon: "folder"
                text: "SOURCE CODE"
                onClicked: Qt.openUrlExternally(AppController.repository())
            }
        }
    }
}
