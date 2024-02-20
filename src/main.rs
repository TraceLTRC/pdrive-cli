use clap::Parser;
use core::{fmt, panic};
use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::File,
    io::{self, BufReader, Read},
    path::PathBuf,
    str::FromStr,
};

const FILE_SPLIT_SIZE: usize = 50 * 1024 * 1024; // 50MB

#[derive(Parser)]
struct Cli {
    file: std::path::PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Config {
    token: String,
    api_url: String,
    concurrent_requests: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            token: String::from_str("MISSING_TOKEN").unwrap(),
            api_url: String::from_str("MISSING_API").unwrap(),
            concurrent_requests: 2,
        }
    }
}

#[derive(Debug, Clone)]
struct ClientError {
    message: String,
}

impl Error for ClientError {}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Server error occured: {}", self.message)
    }
}

#[derive(Deserialize)]
struct R2Multipart {
    key: String,
    #[serde(rename = "uploadId")]
    upload_id: String,
}

#[derive(Serialize, Deserialize)]
struct R2Part {
    #[serde(rename = "partNumber")]
    part_number: u32,
    etag: String,
}

fn split_bytes(path: PathBuf) -> io::Result<Vec<Vec<u8>>> {
    let mut f = BufReader::new(File::open(path)?);
    let mut buffer: Vec<u8> = Vec::with_capacity(FILE_SPLIT_SIZE);
    let mut chunks: Vec<Vec<u8>> = Vec::new();

    loop {
        buffer.clear();
        let n = f
            .by_ref()
            .take(FILE_SPLIT_SIZE as u64)
            .read_to_end(&mut buffer)?;

        if n == 0 {
            break;
        }

        chunks.push(buffer.clone());
    }

    Ok(chunks)
}

async fn single_upload(path: PathBuf, config: &Config) -> Result<String, Box<dyn Error>> {
    let client = reqwest::Client::new();
    let mut file = BufReader::new(File::open(&path)?);
    let mut buf: Vec<_> = Vec::new();

    let _ = file.read_to_end(&mut buf)?;

    println!("Uploading...");

    let res = client
        .post(config.api_url.clone() + "/upload/" + &path.file_name().unwrap().to_str().unwrap())
        .body(buf)
        .bearer_auth(config.token.clone())
        .send()
        .await?;

    match res.status() {
        reqwest::StatusCode::OK => Ok(res.text().await?),
        reqwest::StatusCode::BAD_REQUEST => {
            let message = res.text().await?;
            Err(Box::new(ClientError { message }))
        }
        reqwest::StatusCode::UNAUTHORIZED => Err(Box::new(ClientError {
            message: "Wrong token".to_owned(),
        })),
        _ => panic!("Unexpected status code: {}", res.status()),
    }
}

async fn multipart_upload(
    key: String,
    parts: Vec<Vec<u8>>,
    config: &Config,
) -> Result<String, Box<dyn Error>> {
    // Init part
    let client = reqwest::Client::new();
    // TODO: This request will error weirdly if token is wrong
    println!("Initializing part upload");
    let multipart = client
        .post(config.api_url.clone() + "/upload-part/init/" + &key)
        .bearer_auth(config.token.clone())
        .send()
        .await?
        .json::<R2Multipart>()
        .await?;

    println!("Uploading parts...");
    let parts = stream::iter(parts)
        .enumerate()
        .map(|(index, part)| {
            let client = &client;
            let multipart = &multipart;
            async move {
                println!("Uploading part {}", index + 1);
                let resp = client
                    .put(
                        config.api_url.clone()
                            + "/upload-part/put/"
                            + &multipart.key
                            + "/"
                            + &multipart.upload_id
                            + "?partNumber="
                            + &(index + 1).to_string(),
                    )
                    .bearer_auth(config.token.clone())
                    .body(part)
                    .send()
                    .await?;
                println!("Finished uploading part {}", index + 1);
                if resp.status() != reqwest::StatusCode::OK {
                    let status = resp.status();
                    let text = resp.text().await?;
                    panic!("Unexpected status code ({:?}): {}", status, text)
                }
                resp.json::<R2Part>().await
            }
        })
        .buffer_unordered(2);

    let uploaded_parts = parts.try_collect::<Vec<_>>().await?;

    // Complete part
    println!("Completing part upload...");
    let complete_resp = client
        .post(
            config.api_url.clone()
                + "/upload-part/finish/"
                + &multipart.key
                + "/"
                + &multipart.upload_id,
        )
        .bearer_auth(config.token.clone())
        .json(&uploaded_parts)
        .send()
        .await?;

    match complete_resp.status() {
        reqwest::StatusCode::OK => Ok(complete_resp.text().await?),
        _ => {
            let status = complete_resp.status();
            let text = complete_resp.text().await?;
            panic!("Unexpected status code ({:?}): {}", status, text);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // TODO: Add flag for quite mode
    let args = Cli::parse();
    let config: Config = confy::load("pdrive", None)?;

    let metadata = args.file.metadata()?;
    let bytes = metadata.len();
    if bytes <= FILE_SPLIT_SIZE as u64 {
        let res = single_upload(args.file, &config).await?;
        println!("{}", config.api_url.clone() + "/" + &res);
        Ok(())
    } else {
        let key = args.file.file_name().unwrap().to_str().unwrap();
        let splits = split_bytes(args.file.clone())?;
        let res = multipart_upload(key.to_owned(), splits, &config).await?;
        println!("{}", config.api_url.clone() + "/" + &res);
        Ok(())
    }
}
