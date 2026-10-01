pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import rille.ui

// Dark drop-down list in the app's style.
ComboBox {
    id: combo
    implicitHeight: 24
    font.family: Theme.fontCondensed
    font.pixelSize: Theme.fontNormal
    font.bold: true

    background: Rectangle {
        radius: Theme.radius
        border.color: combo.activeFocus || combo.popup.visible ? Theme.fx : Theme.border
        gradient: Gradient {
            GradientStop { position: 0; color: combo.hovered ? Theme.controlHover : Theme.controlTop }
            GradientStop { position: 1; color: Theme.controlBottom }
        }
    }
    contentItem: UiText {
        leftPadding: 8
        rightPadding: 22
        text: combo.displayText
        font: combo.font
        color: Theme.text
        elide: Text.ElideRight
    }
    indicator: Icon {
        x: combo.width - width - 7
        anchors.verticalCenter: parent.verticalCenter
        name: "chevron-down"
        size: 13
        color: Theme.textDim
    }
    delegate: ItemDelegate {
        id: item
        required property int index
        required property var modelData
        width: combo.width
        height: 26
        highlighted: combo.highlightedIndex === index
        contentItem: UiText {
            text: item.modelData
            font.family: Theme.fontCondensed
            font.bold: item.index === combo.currentIndex
            color: item.index === combo.currentIndex ? Theme.fx : Theme.text
        }
        background: Rectangle { color: item.highlighted ? Theme.selection : Theme.panelRaised }
    }
    popup: Popup {
        y: combo.height + 2
        width: combo.width
        padding: 1
        implicitHeight: Math.min(contentItem.implicitHeight + 2, 320)
        contentItem: ListView {
            clip: true
            implicitHeight: contentHeight
            model: combo.popup.visible ? combo.delegateModel : null
            currentIndex: combo.highlightedIndex
        }
        background: Rectangle { color: Theme.panelRaised; border.color: Theme.border; radius: Theme.radius }
    }
}
