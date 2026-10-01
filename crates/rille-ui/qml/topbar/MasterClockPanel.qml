import QtQuick
import QtQuick.Layouts
import rille.ui

// Above the mixer: snap/quantize, the master tempo and who leads it, main
// level and limiter.
Rectangle {
    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    RowLayout {
        anchors.fill: parent
        anchors.margins: 5
        spacing: 6

        ColumnLayout {
            Layout.fillHeight: true
            Layout.preferredWidth: 62
            spacing: 2
            DjButton { Layout.fillWidth: true; Layout.fillHeight: true; subtle: true; text: "SNAP"; target: "global.snap"; lit: AppController.snap; litColor: Theme.sync; tip: "Snap: new cue points and hotcues land on the nearest beat" }
            DjButton { Layout.fillWidth: true; Layout.fillHeight: true; subtle: true; text: "QUANT"; target: "global.quantize"; lit: AppController.quantize; litColor: Theme.sync; tip: "Quantize: jumps while playing keep the beat phase, so the mix stays in time" }
            DjButton {
                Layout.fillWidth: true
                Layout.fillHeight: true
                subtle: true
                text: AppController.masterDeck < 0 ? "CLOCK" : "AUTO"
                lit: AppController.masterDeck < 0
                tip: "Tempo master: AUTO follows a deck; click to hand it to the internal clock"
                onClicked: AppController.setInternalMaster()
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0
            UiText {
                Layout.alignment: Qt.AlignHCenter
                text: AppController.clockBpm.toFixed(2)
                font.pixelSize: 26
                font.family: Theme.fontMono
                font.weight: Font.Medium
                font.letterSpacing: -0.5
                Tip { text: "Master tempo; the mouse wheel changes the clock tempo in 0.1 BPM steps"; visible: bpmArea.containsMouse }
                MouseArea {
                    id: bpmArea
                    anchors.fill: parent
                    hoverEnabled: true
                    onWheel: wheel => AppController.setClockTempo(AppController.clockBpm + (wheel.angleDelta.y > 0 ? 0.1 : -0.1))
                }
            }
            UiText {
                Layout.alignment: Qt.AlignHCenter
                text: AppController.masterDeck < 0 ? "MASTER: CLOCK" : "MASTER: DECK " + String.fromCharCode(65 + AppController.masterDeck)
                color: Theme.accent
                font.pixelSize: Theme.fontTiny
                font.bold: true
            }
            Item { Layout.fillHeight: true }
            DjButton {
                Layout.fillWidth: true
                subtle: true; text: "LIMITER"
                target: "global.limiter"
                lit: AppController.limiter
                litColor: Theme.warn
                implicitHeight: 20
                tip: "Limiter on the main output: keeps loud mixes from clipping"
            }
        }

        Knob { Layout.alignment: Qt.AlignVCenter; label: "MAIN"; size: 36; value: AppController.mainLevel; defaultValue: 0.8; target: "global.main_level"; color: Theme.text; tip: "Main output level" }
    }
}
