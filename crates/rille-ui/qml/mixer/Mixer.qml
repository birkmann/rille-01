import QtQuick
import QtQuick.Layouts
import rille.ui

// The mixer between the decks. 2 decks: channel A on the left, B mirrored
// on the right, EQ columns and faders meeting in the middle. 4 decks: one
// column per channel in the order C A B D, so each side's decks sit next
// to their side of the deck area.
Rectangle {
    id: mixer
    required property DeckController deckA
    required property DeckController deckB
    required property DeckController deckC
    required property DeckController deckD
    property bool fourDecks: false
    color: Theme.panel
    radius: Theme.radius
    border.color: Theme.panelEdge

    component Separator: Rectangle {
        Layout.fillHeight: true
        Layout.preferredWidth: 1
        Layout.leftMargin: 3
        Layout.rightMargin: 3
        color: Theme.border
    }

    RowLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 0
        visible: !mixer.fourDecks
        ChannelStrip { dc: mixer.deckA; Layout.fillHeight: true; Layout.fillWidth: true }
        Separator {}
        ChannelStrip { dc: mixer.deckB; mirrored: true; Layout.fillHeight: true; Layout.fillWidth: true }
    }

    RowLayout {
        anchors.fill: parent
        anchors.margins: 6
        spacing: 0
        visible: mixer.fourDecks
        ChannelColumn { dc: mixer.deckC; Layout.fillHeight: true; Layout.fillWidth: true; Layout.preferredWidth: 1 }
        Separator {}
        ChannelColumn { dc: mixer.deckA; Layout.fillHeight: true; Layout.fillWidth: true; Layout.preferredWidth: 1 }
        Separator {}
        ChannelColumn { dc: mixer.deckB; Layout.fillHeight: true; Layout.fillWidth: true; Layout.preferredWidth: 1 }
        Separator {}
        ChannelColumn { dc: mixer.deckD; Layout.fillHeight: true; Layout.fillWidth: true; Layout.preferredWidth: 1 }
    }
}
