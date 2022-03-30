use std::pin::Pin;
use tokio_stream::Stream;
use tonic::{transport::Server, Request, Response};
use tracing::debug;

type GrpcResult<T> = std::result::Result<Response<T>, tonic::Status>;
type ResponseStream = Pin<
    Box<dyn Stream<Item = Result<proto::DeviceInfo, tonic::Status>> + Send>,
>;

mod proto {
    tonic::include_proto!("drmem");
}

use proto::dr_mem_server::{DrMem, DrMemServer};

#[derive(Default)]
pub struct DrMemImpl {}

#[tonic::async_trait]
impl DrMem for DrMemImpl {
    #[allow(non_camel_case_types)]
    type getDeviceInfoStream = ResponseStream;

    async fn get_device_info(
        &self, request: Request<proto::Devices>,
    ) -> GrpcResult<Self::getDeviceInfoStream> {
        debug!("request from {:?}", request.remote_addr());
    }

    async fn query_devices(
        &self, request: Request<proto::DeviceFilter>,
    ) -> GrpcResult<proto::Devices> {
        debug!("request from {:?}", request.remote_addr());
        Ok(Response::new(proto::Devices { name: vec![] }))
    }
}

#[tokio::main]
async fn start(
    addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = DrMemImpl::default();

    debug!("gRPC interface listening on {}", addr);

    Server::builder()
        .add_service(DrMemServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
