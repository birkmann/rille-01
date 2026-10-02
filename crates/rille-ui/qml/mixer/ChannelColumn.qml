import QtQuick
import QtQuick.Layouts
import rille.ui

// One channel of the 4-channel mixer in a single column: deck letter,
// GAIN (and KEY beside it when turned on in the settings), the 3-band EQ
// and filter, FX assign and headphone cue, then the channel fader beside
// its level meter. The knobs shrink on short windows so the fader keeps
// its travel.
Item {
    id: strip
    required property DeckController dc

    // External mixing (a hardware mixer such as the Xone:96 does faders and
    // headphones): the mixer channel this deck plays on, 1-based, 0 = none.
    readonly property bool external: AppController.mixerChannels.length > 0
    readonly property int hwChannel: AppController.mixerChannels.indexOf(String.fromCharCode(65 + dc.deck)) + 1

    // Everything but the knobs takes about 221 px (fader at least 90).
    readonly property int knobSize: Math.max(24, Math.min(36, Math.floor((height - 221) / 5)))

    function t(name) {
        return dc.target(name)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 4

        UiText {
            Layout.alignment: Qt.AlignHCenter
            text: String.fromCharCode(65 + strip.dc.deck)
            color: strip.dc.master ? Theme.accent : Theme.textDim
            font.pixelSize: Theme.fontLarge
            font.family: Theme.fontCondensed
            font.bold: true
        }
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: 0
            Knob { size: strip.knobSize - 8; label: "GAIN"; value: strip.dc.gain; target: strip.t("gain"); bipolar: true; format: Theme.gainText }
            Knob { visible: AppController.mixerKey; size: strip.knobSize - 8; label: "KEY"; value: strip.dc.keyShift / 24 + 0.5; target: strip.t("key_shift"); bipolar: true; color: Theme.fx; format: Theme.keyShiftText }
        }
        Knob { Layout.alignment: Qt.AlignHCenter; size: strip.knobSize; label: "HI"; value: strip.dc.eqHi; target: strip.t("eq_hi"); bipolar: true; color: Theme.eqColor(value); format: Theme.eqText }
        Knob { Layout.alignment: Qt.AlignHCenter; size: strip.knobSize; label: "MID"; value: strip.dc.eqMid; target: strip.t("eq_mid"); bipolar: true; color: Theme.eqColor(value); format: Theme.eqText }
        Knob { Layout.alignment: Qt.AlignHCenter; size: strip.knobSize; label: "LO"; value: strip.dc.eqLo; target: strip.t("eq_lo"); bipolar: true; color: Theme.eqColor(value); format: Theme.eqText }
        Knob { Layout.alignment: Qt.AlignHCenter; size: strip.knobSize; label: "FILTER"; value: strip.dc.filter; target: strip.t("filter"); bipolar: true; color: Theme.warn; format: Theme.filterText }
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: 1
            DjButton { text: "1"; target: strip.t("fx_assign.1"); lit: strip.dc.fx1; litColor: Theme.fx; implicitWidth: 20; implicitHeight: 22 }
            DjButton { text: "2"; target: strip.t("fx_assign.2"); lit: strip.dc.fx2; litColor: Theme.fx; implicitWidth: 20; implicitHeight: 22 }
            DjButton { visible: !strip.external; icon: "headphones"; target: strip.t("pfl"); lit: strip.dc.pfl; litColor: Theme.warn; implicitWidth: 23; implicitHeight: 22 }
        }
        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillHeight: true
            spacing: 4
            Rectangle {
                Layout.fillHeight: true
                Layout.preferredWidth: 14
                radius: 2
                color: Theme.bg
                border.color: Theme.panelEdge
                Row {
                    anchors.fill: parent
                    anchors.margins: 2
                    spacing: 2
                    VuMeter { width: 4; height: parent.height; level: strip.dc.meterL }
                    VuMeter { width: 4; height: parent.height; level: strip.dc.meterR }
                }
            }
            // External mixing: the hardware channel this deck plays on.
            ColumnLayout {
                visible: strip.external
                Layout.alignment: Qt.AlignHCenter
                Layout.fillHeight: true
                Layout.preferredWidth: 30
                spacing: 2
                Item { Layout.fillHeight: true }
                UiText { Layout.alignment: Qt.AlignHCenter; text: "CH"; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true; font.family: Theme.fontCondensed }
                UiText {
                    Layout.alignment: Qt.AlignHCenter
                    text: strip.hwChannel > 0 ? strip.hwChannel : "–"
                    color: strip.hwChannel > 0 ? Theme.text : Theme.textDim
                    font.pixelSize: Theme.fontLarge
                    font.family: Theme.fontCondensed
                    font.bold: true
                }
            }
            Fader {
                visible: !strip.external
                Layout.fillHeight: true
                Layout.preferredWidth: 30
                value: strip.dc.volume
                defaultValue: 1.0
                target: strip.t("volume")
                color: Theme.text
            }
        }
    }
}
