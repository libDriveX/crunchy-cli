use crate::utils::context::Context;
use crate::Execute;
use crunchyroll_rs::search::QueryOptions;
use crunchyroll_rs::MediaCollection;
use log::warn;
use serde::Serialize;
use serde_json::Value;
use crate::utils::parse::parse_url;

#[derive(Clone, Debug)]
pub enum QueryType {
    Series,
    Episode,
    Movie,
}

impl QueryType {
    fn parse(s: &str) -> Result<QueryType, String> {
        Ok(match s.to_lowercase().as_str() {
            "series" => QueryType::Series,
            "episode" | "episodes" => QueryType::Episode,
            "movie" | "movies" => QueryType::Movie,
            _ => return Err(format!("'{}' is not a valid query type", s)),
        })
    }
}

#[derive(Clone, Debug)]
pub enum OutputFormat {
    Csv,
    QuotedCsv,
    Json,
}

impl OutputFormat {
    fn parse(s: &str) -> Result<OutputFormat, String> {
        Ok(match s.to_lowercase().as_str() {
            "csv" => OutputFormat::Csv,
            "quoted-csv" | "csv-quoted" => OutputFormat::QuotedCsv,
            "json" => OutputFormat::Json,
            _ => return Err(format!("'{}' is not a valid output format", s)),
        })
    }
}

#[derive(Debug, clap::Parser)]
#[clap(about = "Get information by a word query")]
#[command(arg_required_else_help(true))]
pub struct Query {
    #[arg(help = "Number of results to fetch")]
    #[arg(short = 'n', long, default_value_t = 10)]
    limit: u32,
    #[arg(help = "Type of results to return. \
    Available options are: 'series', 'episodes', 'movies'. \
    None means mixed")]
    #[arg(long)]
    #[arg(value_parser = QueryType::parse)]
    query_type: Option<QueryType>,

    #[arg(long, default_value_t = false)]
    id: bool,
    #[arg(long, default_value_t = false)]
    url: bool,
    #[arg(long = "type", default_value_t = false)]
    type_: bool,
    #[arg(long, default_value_t = false)]
    title: bool,
    #[arg(long, default_value_t = false)]
    description: bool,

    #[arg(help = "Format in which the output should be displayed. \
    Available options are: 'csv' and 'json'")]
    #[arg(long_help = "Format in which the output should be displayed. \
    Available options are: 'csv', 'quoted-csv' and 'json'. \
    Note that 'quoted-csv' will remove all newlines to keep the output parsable")]
    #[arg(long, default_value = "csv")]
    #[arg(value_parser = OutputFormat::parse)]
    output_format: OutputFormat,

    input: String,
}

struct Output {
    id: String,
    url: String,
    type_: String,
    title: String,
    description: String,
}

#[serde_with::skip_serializing_none]
#[derive(Serialize)]
struct FormattedOutput {
    id: Option<String>,
    url: Option<String>,
    #[serde(rename = "type")]
    type_: Option<String>,
    title: Option<String>,
    description: Option<String>,
}

#[async_trait::async_trait(?Send)]
impl Execute for Query {
    async fn execute(self, ctx: Context) -> anyhow::Result<()> {
        let results = if crunchyroll_rs::parse_url(self.input.clone()).is_some() {
            vec![parse_url(&ctx.crunchy, self.input.clone(), true).await?.0]
        } else {
            let mut query_options = QueryOptions::default().limit(self.limit);
            if let Some(query_type) = &self.query_type {
                query_options = match query_type {
                    &QueryType::Series => query_options.result_type(crunchyroll_rs::search::QueryType::Series),
                    &QueryType::Episode => query_options.result_type(crunchyroll_rs::search::QueryType::Episode),
                    &QueryType::Movie => query_options.result_type(crunchyroll_rs::search::QueryType::MovieListing),
                }
            }
            let query = ctx.crunchy.query(&self.input, query_options).await?;
            query.top_results.unwrap().items
        };

        let mut outputs = vec![];

        for result in results {
            match result {
                MediaCollection::Series(series) => outputs.push(Output {
                    id: series.id.clone(),
                    url: format!(
                        "https://www.crunchyroll.com/series/{}/{}",
                        series.id, series.slug_title
                    ),
                    type_: "series".to_string(),
                    title: series.title,
                    description: series.description,
                }),
                MediaCollection::Season(_) => {
                    warn!("Found season, skipping")
                }
                MediaCollection::Episode(episode) => outputs.push(Output {
                    id: episode.id.clone(),
                    url: format!(
                        "https://www.crunchyroll.com/watch/{}/{}",
                        episode.id, episode.slug_title
                    ),
                    type_: "episode".to_string(),
                    title: episode.title,
                    description: episode.description,
                }),
                MediaCollection::MovieListing(movie_listing) => {
                    let movies = movie_listing.movies().await?;
                    if let Some(movie) = movies.get(0) {
                        outputs.push(Output {
                            id: movie.id.clone(),
                            url: format!(
                                "https://www.crunchyroll.com/watch/{}/{}",
                                movie.id, movie.slug_title
                            ),
                            type_: "movie".to_string(),
                            title: movie.title.clone(),
                            description: movie.description.clone(),
                        })
                    } else {
                        warn!("Movie listing queried but no movie found")
                    }
                }
                MediaCollection::Movie(movie) => outputs.push(Output {
                    id: movie.id.clone(),
                    url: format!(
                        "https://www.crunchyroll.com/watch/{}/{}",
                        movie.id, movie.slug_title
                    ),
                    type_: "movie".to_string(),
                    title: movie.title,
                    description: movie.description,
                }),
            }
        }

        for output in convert_to_formatted_outputs(&self, outputs) {
            let as_json = serde_json::to_value(&output)?;
            let as_map = sort_json(as_json.as_object().expect("json object").clone());

            match self.output_format {
                OutputFormat::Csv => {
                    println!(
                        "{}",
                        as_map
                            .values()
                            .into_iter()
                            .map(|v| v.as_str().expect("json string").to_string())
                            .collect::<Vec<String>>()
                            .join(";")
                    )
                }
                OutputFormat::QuotedCsv => {
                    let mut csv = vec![];
                    for value in as_map.values().into_iter() {
                        let mut buf = String::new();
                        buf.push('"');

                        // generate the csv
                        let value_as_string = value.as_str().expect("json string");
                        for char in value_as_string.chars() {
                            if char == '"' {
                                buf.push('"');
                            } else if char == '\r' || char == '\n' {
                                continue;
                            }
                            buf.push(char)
                        }
                        buf.push('"');

                        csv.push(buf)
                    }

                    println!("{}", csv.join(";"))
                }
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string(&as_map)?)
                }
            }
        }

        Ok(())
    }
}

fn convert_to_formatted_outputs(query: &Query, outputs: Vec<Output>) -> Vec<FormattedOutput> {
    let mut format_outputs = vec![];
    for output in outputs {
        format_outputs.push(FormattedOutput {
            id: query.id.then_some(output.id),
            url: query.url.then_some(output.url),
            type_: query.type_.then_some(output.type_),
            title: query.title.then_some(output.title),
            description: query.description.then_some(output.description),
        })
    }
    format_outputs
}

fn sort_json(mut object: serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    let mut sorted = serde_json::Map::with_capacity(object.len());

    for arg in std::env::args() {
        for (key, value) in object.clone() {
            if arg == format!("--{}", &key) {
                object.remove(&key);
                sorted.insert(key, value);
                break;
            }
        }
    }

    sorted
}
