//! Everything related to the websocket connection

use actix::prelude::*;
use actix_web::{
    ws::{Message, ProtocolError, WebsocketContext},
    Binary,
};
use backend::server::{ServerError, State};
use capnp::{
    message::{Builder, ReaderOptions},
    serialize_packed::{read_message, write_message},
};
use failure::Error;
use protocol_capnp::{request, response};

/// The actual websocket
pub struct WebSocket;

impl Actor for WebSocket {
    type Context = WebsocketContext<Self, State>;
}

/// Handler for `Message`
impl StreamHandler<Message, ProtocolError> for WebSocket {
    fn handle(&mut self, msg: Message, ctx: &mut Self::Context) {
        match msg {
            Message::Binary(bin) => if let Err(e) = self.handle_request(&bin, ctx) {
                warn!("Unable to send response: {}", e);
            },
            Message::Close(reason) => {
                info!("Closing websocket connection: {:?}", reason);
                ctx.stop();
            }
            e => warn!("Got invalid message: {:?}", e),
        }
    }
}

impl WebSocket {
    fn handle_request(&mut self, data: &Binary, ctx: &mut WebsocketContext<Self, State>) -> Result<(), Error> {
        // Try to read the message
        let reader = read_message(&mut data.as_ref(), ReaderOptions::new())?;
        let request = reader.get_root::<request::Reader>()?;

        // Create a new message builder
        let mut message = Builder::new_default();
        let mut response_data = Vec::new();

        // Check the request type
        match request.which() {
            Ok(request::Login(data)) => {
                // Check if its a credential or token login type
                match data?.which() {
                    Ok(request::login::Credentials(d)) => {
                        let v = d?;
                        let username = v.get_username()?;
                        let password = v.get_password()?;
                        debug!("User {} is trying to login", username);

                        // Error if username and password are invalid
                        if username.is_empty() || password.is_empty() {
                            debug!("Wrong username or password");
                            return Err(ServerError::WrongUsernamePassword.into());
                        }

                        // Create a new token
                        let token = ctx.state().store.create(username)?;

                        // Create the response
                        message.init_root::<response::Builder>().init_login().set_token(&token);

                        // Write the message into a buffer
                        write_message(&mut response_data, &message)?;

                        // Send the response to the websocket
                        ctx.binary(response_data);
                        Ok(())
                    }
                    Ok(request::login::Token(token_data)) => {
                        let token = token_data?;
                        debug!("Token {} wants to be renewed", token);

                        // Try to verify and create a new token
                        match ctx.state().store.verify(token) {
                            Ok(new_token) => {
                                // Create the success response
                                message
                                    .init_root::<response::Builder>()
                                    .init_login()
                                    .set_token(&new_token);
                            }
                            Err(e) => {
                                // Create the failure response
                                message
                                    .init_root::<response::Builder>()
                                    .init_login()
                                    .set_error(&e.to_string());
                            }
                        }

                        // Write the message into a buffer
                        write_message(&mut response_data, &message)?;

                        // Send the response to the websocket
                        ctx.binary(response_data);
                        Ok(())
                    }
                    Err(e) => Err(e.into()),
                }
            }
            Ok(request::Logout(token)) => {
                ctx.state().store.remove(token?)?;
                message.init_root::<response::Builder>().init_logout().set_success(());
                write_message(&mut response_data, &message)?;
                ctx.binary(response_data);
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }
}