use std::time::Duration;

/// Want to test
///  - connect, give connection, send data
///  - don't connect, give connector, send data (with and without autoconnect)
///  - connect, give connection, send data, expire link, (test grace period,
///    reconnection, and neither) make sure that epoch advances in this case
///  - Make tests for each of the time out conditions
///
///
///  - Test by adding new, but quickly expiring connections to make sure that
///    the epoch does not advance
///  - Test that high latency connections are removed
///  - Test that messages are resent when parts are dropped
use link_set::{
    LinkSet, LinkSetMessage, LinkSetSendable,
    links::{PipeLinkBuilder, PipeLinkHub},
};
use tokio::time::sleep;
use tracing::trace;

const SIDE_1_ADDR: &str = "addr1";
const SIDE_2_ADDR: &str = "addr2";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Data(String);

impl LinkSetSendable for Data {
    type E = std::string::FromUtf8Error;

    fn to_bytes(self) -> Result<Vec<u8>, Self::E> {
        Ok(self.0.as_bytes().to_vec())
    }

    fn from_bytes(bytes: Vec<u8>) -> Result<Self, Self::E>
    where
        Self: Sized,
    {
        Ok(Data(String::from_utf8(bytes)?))
    }
}

async fn create_default_config(
    enable_a_connector: bool,
    enable_b_connector: bool,
) -> (PipeLinkHub, LinkSet<Data>, LinkSet<Data>) {
    // Create the hub
    let builder = PipeLinkBuilder::new();
    let mut hub = PipeLinkHub::new(builder);

    // Create the first link set
    let link_set1 = LinkSet::<Data>::new();

    // Create listener
    let mut listener1 = hub.listen(SIDE_1_ADDR.to_owned());
    let link_set1_sender = link_set1.clone_sender();
    tokio::spawn(async move {
        while let Some(link) = listener1.recv().await {
            trace!("Link a got link from listener");
            let _ = link_set1_sender.add_link(Box::new(link)).await;
        }
    });

    // Create connector
    if enable_a_connector {
        let hub1 = hub.clone();
        link_set1
            .add_connector(move |addr: String| {
                trace!("Link a is connecting");
                let mut hub_clone = hub1.clone();
                async move { hub_clone.connect(&addr).ok_or(link_set::LinkSetError::Closed) }
            })
            .await
            .expect("link should stay alive long enough to add the connector");
        link_set1.add_addr(SIDE_2_ADDR.to_owned()).await.unwrap();
    }

    // Create the second link set
    let link_set2 = LinkSet::<Data>::new();

    // Create listener
    let mut listener2 = hub.listen(SIDE_2_ADDR.to_owned());
    let link_set2_sender = link_set2.clone_sender();
    tokio::spawn(async move {
        while let Some(link) = listener2.recv().await {
            trace!("Link b got link from listener");
            let _ = link_set2_sender.add_link(Box::new(link)).await;
        }
    });

    // Create connector
    if enable_b_connector {
        let hub2 = hub.clone();
        link_set2
            .add_connector(move |addr: String| {
                trace!("Link b is connecting");
                let mut hub_clone = hub2.clone();
                async move { hub_clone.connect(&addr).ok_or(link_set::LinkSetError::Closed) }
            })
            .await
            .expect("link should stay alive long enough to add the connector");
        link_set2.add_addr(SIDE_1_ADDR.to_owned()).await.unwrap();
    }

    (hub, link_set1, link_set2)
}

#[tokio::test]
#[test_log::test]
async fn pipe_link_set_send_recv() {
    trace!("running test......");
    let (mut hub, mut a, mut b) = create_default_config(false, false).await;

    // get connection from hub
    let link = hub
        .connect(SIDE_2_ADDR)
        .expect("Failed to get connection from hub");
    a.add_link(Box::new(link)).await.unwrap();

    // both sides should emit connected
    let LinkSetMessage::Connected(_e) = a.recv().await.unwrap() else {
        panic!("Side a did not connect")
    };
    let LinkSetMessage::Connected(_e) = b.recv().await.unwrap() else {
        panic!("Side b did not connect")
    };

    // Send msg through channel
    let msg = Data(String::from("Test Message!"));
    a.send(msg.clone())
        .await
        .expect("Side a should be able to send the message");
    let LinkSetMessage::Message(recvd_msg, _e) = b.recv().await.unwrap() else {
        panic!("Side b did not emit msg")
    };

    // make sure both sides are equal
    assert_eq!(msg, recvd_msg, "Sent and recvd messages should equal");
}

#[tokio::test]
#[test_log::test]
async fn pipe_link_set_connection_lost() {
    let (mut hub, mut a, mut b) = create_default_config(false, false).await;

    // get connection from hub
    let link = hub
        .connect(SIDE_2_ADDR)
        .expect("Failed to get connection from hub");
    let canceler = link.get_canceler();
    a.add_link(Box::new(link)).await.unwrap();

    // both sides should emit connected
    let LinkSetMessage::Connected(_e) = a.recv().await.unwrap() else {
        panic!("Side a did not connect")
    };
    let LinkSetMessage::Connected(_e) = b.recv().await.unwrap() else {
        panic!("Side b did not connect")
    };

    // kill the channel
    canceler.cancel();

    // send message to trigger link closed detection
    let msg = Data(String::from("Test Message!"));
    a.send(msg.clone())
        .await
        .expect("Side a should be able to send the message");

    // both sides should emit disconnected
    let LinkSetMessage::Disconnected = a.recv().await.unwrap() else {
        panic!("Side a did not disconnect")
    };
    let LinkSetMessage::Disconnected = b.recv().await.unwrap() else {
        panic!("Side b did not disconnect")
    };
}

#[tokio::test]
#[test_log::test]
async fn pipe_link_set_connect_on_msg() {
    let (_hub, mut a, mut b) = create_default_config(true, false).await;

    // send message to trigger auto connection
    let msg = Data(String::from("Test Message!"));
    a.send(msg.clone())
        .await
        .expect("Side a should be able to send the message");

    // both sides should emit connected
    let LinkSetMessage::Connected(_e) = a.recv().await.unwrap() else {
        panic!("Side a did not connect")
    };
    let LinkSetMessage::Connected(_e) = b.recv().await.unwrap() else {
        panic!("Side b did not connect")
    };

    let LinkSetMessage::Message(recvd_msg, _e) = b.recv().await.unwrap() else {
        panic!("Side b did not emit msg")
    };
    assert_eq!(msg, recvd_msg, "sent and received messages should match");
}


// Create more lifecycle tests


// Stability testing

// - Expiration
#[tokio::test]
#[test_log::test]
async fn pipe_link_expire_5s() {
    let (mut hub, mut a, mut b) = create_default_config(true, false).await;
    hub.modify_builder(|builder| builder.expiration(Some(Duration::from_secs(5))) );
    a.set_auto_connect(true).await.unwrap();
    a.set_reconnection_timeout(Some(Duration::from_secs(10))).await.unwrap();
    b.set_auto_connect(false).await.unwrap();
    b.set_grace_period_timeout(Some(Duration::from_secs(10))).await.unwrap();

    // connect both sides
    a.connect().await.unwrap();
    b.connect().await.unwrap();
    let LinkSetMessage::Connected(_e) = a.recv().await.unwrap() else {
        panic!("Side a did not connect")
    };
    let LinkSetMessage::Connected(_e) = b.recv().await.unwrap() else {
        panic!("Side b did not connect")
    };

    // create messages
    let mut msgs = Vec::new();
    for i in 0..30 {
        msgs.push(Data(format!("Message {}", i)));
    }

    // start listening for msgs
    let recv_handle = tokio::spawn(async move {
        let mut msgs = Vec::new();
        while let Ok(msg) = b.recv().await {
            let should_break = if let LinkSetMessage::Disconnected = msg {
                true
            }else{
                false
            };

            msgs.push(msg);
            if should_break {
                break;
            }
        }
        msgs
    });

    // Send one msg every 1 second
    for msg in msgs.iter().cloned() {
        a.send(msg.clone())
            .await
            .expect("Side a should be able to send the message");
        sleep(Duration::from_secs(1)).await;
    }

    // Then disconnect
    a.disconnect().await.unwrap();

    // get msgs from receiver
    let recvd_msgs = recv_handle.await.unwrap();
    let mut recvd_iter = recvd_msgs.iter();

    // Check recvd msgs
    for msg in msgs.iter() {
        let Some(LinkSetMessage::Message(recvd_msg, _e)) = recvd_iter.next() else {
            panic!("Side b did not emit msg")
        };
        trace!("Received message {:?}", recvd_msg.0);
        assert_eq!(msg, recvd_msg, "sent and received messages should match");
    }
}

// #[tokio::test]
// #[test_log::test]
// async fn pipe_link_expire_5s_constrained() {
//     let (mut hub, mut a, mut b) = create_default_config(true, false).await;
//     hub.modify_builder(|builder| builder.expiration(Some(Duration::from_secs(5))) );
//     a.set_auto_connect(true).await.unwrap();
//     a.set_reconnection_timeout(Some(Duration::from_secs(10))).await.unwrap();
//     b.set_auto_connect(true).await.unwrap();
//     b.set_grace_period_timeout(Some(Duration::from_secs(10))).await.unwrap();

//     // connect both sides
//     a.connect().await.unwrap();
//     b.connect().await.unwrap();
//     let LinkSetMessage::Connected(_e) = a.recv().await.unwrap() else {
//         panic!("Side a did not connect")
//     };
//     let LinkSetMessage::Connected(_e) = b.recv().await.unwrap() else {
//         panic!("Side b did not connect")
//     };

//     // create messages
//     let mut msgs = Vec::new();
//     for i in 0..30 {
//         msgs.push(Data(format!("Message {}", i)));
//     }

//     // Send one msg every 1 second
//     for msg in msgs.iter().cloned() {
//         a.send(msg.clone())
//             .await
//             .expect("Side a should be able to send the message");
//         sleep(Duration::from_secs(1)).await;
//     }

//     // Check recvd msgs
//     for msg in msgs.iter() {
//         let LinkSetMessage::Message(recvd_msg, _e) = b.recv().await.unwrap() else {
//             panic!("Side b did not emit msg")
//         };
//         trace!("Received message {:?}", recvd_msg.0);
//         assert_eq!(*msg, recvd_msg, "sent and received messages should match");
//     }
// }

// - Reliability
#[tokio::test]
#[test_log::test]
async fn pipe_link_95_loss_50_msg() {
    let (mut hub, mut a, mut b) = create_default_config(true, false).await;
    hub.modify_builder(|builder| builder.reliability(0.95));

    // connect both sides
    a.connect().await.unwrap();
    b.connect().await.unwrap();
    let LinkSetMessage::Connected(_e) = a.recv().await.unwrap() else {
        panic!("Side a did not connect")
    };
    let LinkSetMessage::Connected(_e) = b.recv().await.unwrap() else {
        panic!("Side b did not connect")
    };

    // create messages
    let mut msgs = Vec::new();
    for i in 0..50 {
        msgs.push(Data(format!("Message {}", i)));
    }

    // Send messages
    for msg in msgs.iter().cloned() {
        a.send(msg.clone())
            .await
            .expect("Side a should be able to send the message");
    }

    for msg in msgs.iter() {
        let LinkSetMessage::Message(recvd_msg, _e) = b.recv().await.unwrap() else {
            panic!("Side b did not emit msg")
        };
        assert_eq!(*msg, recvd_msg, "sent and received messages should match");
    }
}

// - Latency
