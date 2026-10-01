import QtQuick
import QtQuick.Layouts
import rille.ui

// One mixer channel: outer column gain, filter, FX assign, key and
// headphone cue; the channel's level meter; inner column the 3-band EQ
// above the channel fader. `mirrored` for the right-hand channel, so both
// EQ/fader columns sit in the middle of the mixer.
Item {
    id: strip
    required property DeckController dc
    property bool mirrored: false

    // External mixing (a hardware mixer such as the Xone:96 does faders and
    // headphones): the mixer channel this deck plays on, 1-based, 0 = none.
    readonly property bool external: AppController.mixerChannels.length > 0
    readonly property int hwChannel: AppController.mixerChannels.indexOf(String.fromCharCode(65 + dc.deck)) + 1

    function t(name) {
        return dc.target(name)
    }

    RowLayout {
        anchors.fill: parent
        spacing: 3
        LayoutMirroring.enabled: strip.mirrored
        LayoutMirroring.childrenInherit: false

        // Outer column.
        ColumnLayout {
            Layout.fillHeight: true
            Layout.preferredWidth: 46
            spacing: 4
            Knob {
                Layout.alignment: Qt.AlignHCenter
                label: "GAIN"
                tip: "Gain: channel level before the EQ"
                value: strip.dc.gain
                target: strip.t("gain")
                bipolar: true
                format: Theme.gainText
            }
            Knob {
                Layout.alignment: Qt.AlignHCenter
                label: "FILTER"
                tip: "Filter: left = low-pass, right = high-pass, centre = off"
                value: strip.dc.filter
                target: strip.t("filter")
                bipolar: true
                color: Theme.warn
                format: Theme.filterText
            }
            RowLayout {
                Layout.alignment: Qt.AlignHCenter
                spacing: 2
                DjButton { text: "1"; target: strip.t("fx_assign.1"); lit: strip.dc.fx1; litColor: Theme.fx; implicitWidth: 22; implicitHeight: 22; tip: "Send this channel through FX unit 1" }
                DjButton { text: "2"; target: strip.t("fx_assign.2"); lit: strip.dc.fx2; litColor: Theme.fx; implicitWidth: 22; implicitHeight: 22; tip: "Send this channel through FX unit 2" }
            }
            UiText { Layout.alignment: Qt.AlignHCenter; text: "FX"; color: Theme.textDim; font.pixelSize: Theme.fontTiny; font.bold: true; font.family: Theme.fontCondensed }
            Knob {
                Layout.alignment: Qt.AlignHCenter
                label: "KEY"
                tip: "Key shift: transpose in semitones, the tempo stays"
                value: strip.dc.keyShift / 24 + 0.5
                target: strip.t("key_shift")
                bipolar: true
                color: Theme.fx
                format: Theme.keyShiftText
            }
            Item { Layout.fillHeight: true }
            DjButton {
                visible: !strip.external
                Layout.alignment: Qt.AlignHCenter
                icon: "headphones"
                target: strip.t("pfl")
                lit: strip.dc.pfl
                litColor: Theme.warn
                implicitWidth: 46
                implicitHeight: 26
                tip: "Headphone cue: hear this channel in the headphones"
            }
        }

        // Level meter, L and R.
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 16
            radius: 2
            color: Theme.bg
            border.color: Theme.panelEdge
            Row {
                anchors.fill: parent
                anchors.margins: 2
                spacing: 2
                VuMeter { width: 5; height: parent.height; level: strip.dc.meterL }
                VuMeter { width: 5; height: parent.height; level: strip.dc.meterR }
            }
        }

        // Inner column: EQ, then the channel fader.
        ColumnLayout {
            Layout.fillHeight: true
            Layout.preferredWidth: 46
            spacing: 4
            Knob { Layout.alignment: Qt.AlignHCenter; label: "HI"; value: strip.dc.eqHi; target: strip.t("eq_hi"); bipolar: true; color: Theme.eqColor(value); format: Theme.eqText }
            Knob { Layout.alignment: Qt.AlignHCenter; label: "MID"; value: strip.dc.eqMid; target: strip.t("eq_mid"); bipolar: true; color: Theme.eqColor(value); format: Theme.eqText }
            Knob { Layout.alignment: Qt.AlignHCenter; label: "LO"; value: strip.dc.eqLo; target: strip.t("eq_lo"); bipolar: true; color: Theme.eqColor(value); format: Theme.eqText }
            // External mixing: the hardware channel this deck plays on.
            ColumnLayout {
                visible: strip.external
                Layout.alignment: Qt.AlignHCenter
                Layout.fillHeight: true
                Layout.preferredWidth: 34
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
                Layout.alignment: Qt.AlignHCenter
                Layout.fillHeight: true
                Layout.preferredWidth: 34
                value: strip.dc.volume
                defaultValue: 1.0
                target: strip.t("volume")
                color: Theme.text
                tip: "Channel fader"
            }
        }
    }
}
