#![feature(libc)]
extern crate libc;
extern crate strfmt;
use strfmt::strfmt;
use std::env;
use actix_web::{get, web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use jsonwebtoken::{decode ,decode_header,  Algorithm, DecodingKey, Validation};
use std::process::Command;
use std::time::{SystemTime};
use chrono::offset::Utc;
use chrono::DateTime;
use rand::distributions::{Alphanumeric, DistString};
use reqwest::header::{HeaderMap};

// need to change this later when load balancer giving all correct IP's
static RTMP_OUT_LOCATION: &str = "rtmp://43.205.21.202:1935/{app}/{stream}";

static DASH_URL: &str = "https://edge.sariska.io/play/dash/{app}/{stream}.mpd";
static HLS_URL: &str = "https://edge.sariska.io/play/hls/{app}/{stream}.m3u8";
static MP3_URL: &str = "https://edge.sariska.io/play/dash/{app}/{stream}.mp3";
static AAC_URL: &str = "https://edge.sariska.io/play/dash/{app}/{stream}.aac";
static RTMP_URL: &str = "rtmp://a0f32a67911bd43b08097a2a99e6eac6-b0099fdbb77fd73a.elb.ap-south-1.amazonaws.com:1935/{app}/{stream}";
static FLV_URL: &str = "https://edge.sariska.io/play/dash/{app}/{stream}.flv";

static GST_MEET_PARAMS_LIVESTREAM: &str = "./gst-meet --web-socket-url=wss://api.sariska.
o/api/v1/media/websocket \
--xmpp-domain=sariska.io  --muc-domain=muc.sariska.io \
 --recv-video-scale-width=640 \
 --recv-video-scale-height=360 \
 --room-name={roomname}  \
 --recv-pipeline='compositor name=video sink_1::xpos=640 \
    ! queue \
    ! x264enc cabac=1 bframes=2 ref=1 \
    ! video/x-h264,profile=main \
    ! flvmux streamable=true name=mux \
    ! rtmpsink location={rtmp_out_location} \
    audiotestsrc is-live=1 wave=ticks \
       ! mux.'";
use std::{collections::HashMap, sync::RwLock};
use libc::{kill, SIGTERM};

// This struct represents state
#[derive(Clone)]
pub struct AppState {
    pub map: HashMap<String,  String>
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Context {
    pub group: String
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    pub context: Context
}

#[derive(Serialize, Deserialize, Debug)]
struct PublicKey {
    e: String,
    n: String,
    kty: String
}

#[derive(Debug, Deserialize)]
struct Params {
    room_name: String,
}

fn print_type_of<T>(_: &T) {
    println!("{}", std::any::type_name::<T>())
}

#[derive(Serialize)]
struct ResponseStart {
    started: bool,
    hls_url:  String,
    dash_url: String,
    mp3_url: String,
    aac_url: String,
    rtmp_url: String,
    flv_url: String
}

#[derive(Serialize)]
struct ResponseStop {
    stopped: bool
}

fn systemtime_strftime(dt: SystemTime, format: &str) -> String {
    let datetime: DateTime<Utc> = dt.into();
    format!("{}", datetime.format(format))
}


async fn sendDataToPricingService(room_name: String, action: String, authorization_header: String) {
    let mut map = HashMap::new();
    let st = SystemTime::now();
    let st_str: String=  systemtime_strftime(st, "%Y-%m-%dT%TZ");
    map.insert("roomJid", format!("{}@muc.sariska.io", room_name));
    map.insert("timestamp",  st_str);
    map.insert("action", action);
    map.insert("type", "stream".to_owned());            

    let serviceSecretKey = match env::var_os("X-SERVICE-TOKEN") {
        Some(v) => v.into_string().unwrap(),
        None => panic!("$X-SERVICE-TOKEN is not set")
    };

    let mut headers = HeaderMap::new();
        headers.insert("Authorization", authorization_header.parse().unwrap());
        headers.insert("X-SERVICE-TOKEN", serviceSecretKey.parse().unwrap());

    let client = reqwest::Client::new();
    let res = client.post("https://api.sariska.io/api/v1/pricing/recording")
        .headers(headers)
        .json(&map)
        .send()
        .await;
}

#[get("/startRecording")]
async fn start_recorging(_req: HttpRequest, child_processes: web::Data<RwLock<AppState>>) -> HttpResponse {
    let params = web::Query::<Params>::from_query(_req.query_string()).unwrap();
    println!("{:?}", params);

    let app: String =  Alphanumeric.sample_string(&mut rand::thread_rng(), 16);
    let stream: String =  Alphanumeric.sample_string(&mut rand::thread_rng(), 16);
    let location = format!("{}",strfmt!(RTMP_OUT_LOCATION, app.to_string(), stream.to_string()).unwrap());
    let pipeline = format!("{}",strfmt!(GST_MEET_PARAMS_LIVESTREAM, params.room_name.to_owned(), location).unwrap());
    let _auth = _req.headers().get("Authorization");
    let _split: Vec<&str> = _auth.unwrap().to_str().unwrap().split("Bearer").collect();
    let token = _split[1].trim();
    let header  =  decode_header(&token);
    let request_url = env::var("SECRET_MANAGEMENT_SERVICE_PUBLIC_KEY_URL").unwrap_or("none".to_string());  
    let kid = "";

   let headerData = match header {
        Ok(_token) => _token.kid,
        Err(_e) => None,
    };
    let kid = headerData.as_deref().unwrap_or("default string");
        // create a Sha256 object
    let api_key_url =  format!("{}/{}", request_url, kid);
    let response = reqwest::get(api_key_url)
        .await
        .unwrap()
        .text()
        .await;
    let publicKey  = match response {
        Ok(_publickey)=>_publickey,
        _ => "default string".to_string(),
    };
    
    let deserialized: PublicKey = serde_json::from_str(&publicKey).unwrap();
    let decoded_claims = decode::<Claims>(
        &token,
        &DecodingKey::from_rsa_components(&"deserialized.n", &deserialized.e),
        &Validation::new(Algorithm::RS256));

        let decoded;
        let mut error = false;
        match decoded_claims {
            Ok(v) => {
                decoded = v;
            },
            Err(e) => {
              error = true;
            },
        }
        
        if error == true {
            println!("unauthorized");
            return HttpResponse::Unauthorized().json("{}");
        }

        let child = Command::new("sh")
        .arg("-c")
        .arg(GST_MEET_PARAMS_LIVESTREAM)
        .spawn()
        .expect("failed to execute process");
         child_processes.write().unwrap().map.insert(params.room_name.to_string(), child.id().to_string());
        // child_processes.insert(params.room_name.to_string(),child);

        sendDataToPricingService(params.room_name.to_string(), "start".to_owned(), token.to_owned()).await;
        
        let obj = createResponseStart(app.clone(), stream.clone());
        HttpResponse::Ok().json(obj)

}

fn createResponseStart(app :String, stream: String) -> ResponseStart {
    let obj = ResponseStart {
        started: true,
        hls_url: format!("{}",strfmt!(HLS_URL, app.to_string(), stream.to_string()).unwrap()),
        dash_url: format!("{}",strfmt!(DASH_URL, app.to_string(), stream.to_string()).unwrap()),
        mp3_url: format!("{}",strfmt!(MP3_URL, app.to_string(), stream.to_string()).unwrap()),
        aac_url: format!("{}",strfmt!(AAC_URL, app.to_string(), stream.to_string()).unwrap()),
        rtmp_url: format!("{}",strfmt!(RTMP_URL, app.to_string(), stream.to_string()).unwrap()),
        flv_url: format!("{}",strfmt!(FLV_URL, app.to_string(), stream.to_string()).unwrap())
    };
    obj
}

#[get("/stopRecording")]
async fn stop_recording(_req: HttpRequest, child_processes: web::Data<RwLock<AppState>>) -> HttpResponse {
    let params = web::Query::<Params>::from_query(_req.query_string()).unwrap();
    let _auth = _req.headers().get("Authorization");
    let _split: Vec<&str> = _auth.unwrap().to_str().unwrap().split("Bearer").collect();
    let token = _split[1].trim();

    let child_ids = &child_processes.read().unwrap().map;
    let child_os_id = child_ids.get(&params.room_name.to_string());  
    let my_int = child_os_id.unwrap().parse::<i32>().unwrap();
    unsafe {
        kill(my_int, SIGTERM);
    }
    sendDataToPricingService(params.room_name.to_string(), "stop".to_owned(), token.to_owned()).await;
    
    let obj = ResponseStop {
        stopped: true,
    };
    HttpResponse::Ok().json(obj)
}

pub fn init_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(start_recorging);
    cfg.service(stop_recording);
}

