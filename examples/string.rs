use std::{io::Write, time::SystemTime};

fn build_string_message(data: &str) -> anyhow::Result<Vec<u8>> {
    let mut msg = vec![0; std::mem::size_of::<u32>() + data.len()];
    // ROS 1 message strings are encoded as 4-bytes length and then the byte data.
    let mut w = std::io::Cursor::new(&mut msg);
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data.as_bytes())?;
    Ok(msg)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let server = foxglove_ws::FoxgloveWebSocket::default();

    let urdf_robot = urdf_rs::Robot {
        name: "base".to_string(),
        joints: vec![urdf_rs::Joint {
            name: "joint".to_string(),
            joint_type: urdf_rs::JointType::Continuous,
            origin: urdf_rs::Pose {
                xyz: urdf_rs::Vec3([0.0, 0.0, 0.0]),
                rpy: urdf_rs::Vec3([0.0, 0.0, 0.0]),
            },
            parent: urdf_rs::LinkName {
                link: "base".to_string(),
            },
            child: urdf_rs::LinkName {
                link: "link".to_string(),
            },
            axis: urdf_rs::Axis {
                xyz: urdf_rs::Vec3([0.0, 0.0, 1.0]),
            },
            limit: urdf_rs::JointLimit {
                lower: -std::f64::consts::PI,
                upper: std::f64::consts::PI,
                effort: 0.0,
                velocity: 0.0,
            },
            dynamics: Some(urdf_rs::Dynamics {
                damping: 0.0,
                friction: 0.0,
            }),
            mimic: None,
            safety_controller: None,
        }],
        links: vec![urdf_rs::Link {
            name: "link".to_string(),
            inertial: urdf_rs::Inertial {
                origin: urdf_rs::Pose {
                    xyz: urdf_rs::Vec3([0.2, 0.2, 0.2]),
                    rpy: urdf_rs::Vec3([0.0, 0.0, 0.0]),
                },
                mass: urdf_rs::Mass { value: 1.0 },
                inertia: urdf_rs::Inertia::default(),
            },
            visual: vec![],
            collision: vec![],
        }],
        materials: vec![],
    };

    let urdf_string = urdf_rs::write_to_string(&urdf_robot)?;

    server
        .parameters
        .write()
        .await
        .insert("/robot_description ".to_string(), urdf_string);

    tokio::spawn({
        let server = server.clone();
        async move { server.serve(([127, 0, 0, 1], 8765)).await }
    });
    let channel = server
        .create_publisher(
            "/data",
            "ros1",
            "std_msgs/String",
            "string data",
            Some("ros1msg"),
            false,
        )
        .await?;
    let channel_latching = server
        .create_publisher(
            "/data_latching",
            "ros1",
            "std_msgs/String",
            "string data",
            Some("ros1msg"),
            true,
        )
        .await?;

    channel_latching
        .send(
            SystemTime::now().elapsed().unwrap().as_nanos() as u64,
            &build_string_message("latching!")?,
        )
        .await?;

    let mut counter = 0;
    loop {
        channel
            .send(
                SystemTime::now().elapsed().unwrap().as_nanos() as u64,
                &build_string_message(&format!("Hello {}!", counter))?,
            )
            .await?;
        counter += 1;
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
}
