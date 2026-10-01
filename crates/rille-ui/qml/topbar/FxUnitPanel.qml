pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import rille.ui

// One FX unit in group mode: on, dry/wet, three effect slots.
Rectangle {
    id: panel
    property int unitIndex: 0
    readonly property string prefix: "fx." + (unitIndex + 1) + "."
    readonly property var fx: {
        var all = JSON.parse(AppController.fxJson || "[]")
        return all.length > unitIndex ? all[unitIndex] : ({ on: false, dryWet: 0.5, effects: [0, 0, 0], knobs: [0, 0, 0], buttons: [false, false, false], names: ["", "", ""] })
    }
    readonly property var effectNames: JSON.parse(AppController.effectNamesJson())

    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    RowLayout {
        anchors.fill: parent
        anchors.margins: 5
        spacing: 8

        ColumnLayout {
            spacing: 4
            UiText { text: "FX " + (panel.unitIndex + 1); color: Theme.fx; font.pixelSize: Theme.fontLarge; font.bold: true; font.family: Theme.fontCondensed }
            DjButton { text: "ON"; target: panel.prefix + "on"; lit: panel.fx.on; litColor: Theme.fx; tip: "FX unit " + (panel.unitIndex + 1) + " on / off (assign channels with the 1 / 2 buttons in the mixer)" }
        }
        Knob {
            label: "D/W"
            size: 34
            value: panel.fx.dryWet
            defaultValue: 0.5
            target: panel.prefix + "dry_wet"
            tip: "Dry/wet: how much of the effected signal is heard"
            color: Theme.fx
        }
        Repeater {
            model: 3
            ColumnLayout {
                id: slot
                required property int index
                spacing: 2
                Layout.fillWidth: true
                StyledCombo {
                    id: combo
                    Layout.fillWidth: true
                    model: panel.effectNames
                    currentIndex: panel.fx.effects[slot.index]
                    onActivated: idx => {
                        var d = idx - panel.fx.effects[slot.index]
                        for (var i = 0; i < Math.abs(d); i++)
                            AppController.nudge(panel.prefix + "select." + (slot.index + 1), d > 0 ? 1 : -1)
                    }
                }
                RowLayout {
                    spacing: 6
                    Knob {
                        size: 28
                        value: panel.fx.knobs[slot.index]
                        target: panel.prefix + "knob." + (slot.index + 1)
                        tip: "Amount of " + (panel.fx.names[slot.index] || "this effect")
                        color: Theme.fx
                    }
                    DjButton {
                        implicitHeight: 24
                        text: "ON"
                        target: panel.prefix + "button." + (slot.index + 1)
                        lit: panel.fx.buttons[slot.index]
                        tip: (panel.fx.names[slot.index] || "Effect") + " on / off"
                        litColor: Theme.fx
                    }
                }
            }
        }
    }
}
