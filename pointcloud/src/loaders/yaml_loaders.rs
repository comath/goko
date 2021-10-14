use glob::{glob_with, MatchOptions};
use std::cmp::Ordering;
use std::fs;
use yaml_rust::YamlLoader;

use log::{info, trace};

use super::*;
use crate::metrics::L2;
use crate::DefaultLabeledCloud;

/// Given a yaml file on disk, it builds a point cloud. Minimal example below.
/// ```yaml
/// ---
/// data_path: DATAMEMMAP
/// labels_path: LABELS_CSV
/// count: NUMBER_OF_DATA_POINTS
/// data_dim: 784
/// label_csv_index: 2
/// ```
pub fn labeled_ram_from_yaml<P: AsRef<Path>, M: Metric<[f32]>>(
    path: P,
) -> PointCloudResult<DefaultLabeledCloud<M>> {
    let label_set = labels_from_yaml(&path)?;
    let data_set = ram_from_yaml(&path)?;

    Ok(SimpleLabeledCloud::new(data_set, label_set))
}

/// Given a yaml file on disk, it builds a point cloud. Minimal example below.
/// ```yaml
/// ---
/// data_path: DATAMEMMAP
/// labels_path: LABELS_MEMMAP
/// count: NUMBER_OF_DATA_POINTS
/// data_dim: 784
/// label_dim: 10
/// ```
pub fn vec_labeled_ram_from_yaml<P: AsRef<Path>, M: Metric<[f32]>>(
    path: P,
) -> PointCloudResult<SimpleLabeledCloud<DataRam<M>, VecLabels>> {
    info!(
        "Opening labeled pointcloud yaml with path {:?}",
        &path.as_ref()
    );
    let config = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Unable to read config file {:?}", &path.as_ref()));

    let params_files = &YamlLoader::load_from_str(&config).unwrap()[0];

    let data_paths = &get_file_list(
        params_files["data_path"]
            .as_str()
            .expect("Unable to read the 'data_path'"),
        path.as_ref(),
    );
    let labels_path = &get_file_list(
        params_files["labels_path"]
            .as_str()
            .expect("Unable to read the 'labels_path'"),
        path.as_ref(),
    );

    let data_dim = params_files["data_dim"]
        .as_i64()
        .expect("Unable to read the 'data_dim'") as usize;

    let labels_dim = params_files["labels_dim"]
        .as_i64()
        .expect("Unable to read the 'labels_dim'") as usize;

    let label_set = convert_glued_memmap_to_ram::<L2>(open_memmaps(labels_dim, labels_path)?)
        .convert_to_labels();
    let data_set = convert_glued_memmap_to_ram(open_memmaps(data_dim, data_paths)?);

    Ok(SimpleLabeledCloud::new(data_set, label_set))
}

/// Given a yaml file on disk, it builds a point cloud. Minimal example below.
/// ```yaml
/// ---
/// data_path: DATAMEMMAP
/// count: NUMBER_OF_DATA_POINTS
/// data_dim: 784
/// ```
pub fn ram_from_yaml<P: AsRef<Path>, M: Metric<[f32]>>(path: P) -> PointCloudResult<DataRam<M>> {
    info!(
        "Opening unlabeled pointcloud yaml with path {:?}",
        &path.as_ref()
    );
    let config = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Unable to read config file {:?}", &path.as_ref()));

    let params_files = &YamlLoader::load_from_str(&config).unwrap()[0];

    let data_paths = &get_file_list(
        params_files["data_path"]
            .as_str()
            .expect("Unable to read the 'data_path'"),
        path.as_ref(),
    );

    let data_dim = params_files["data_dim"]
        .as_i64()
        .expect("Unable to read the 'data_dim'") as usize;

    let data_set = open_memmaps(data_dim, data_paths)?;
    Ok(convert_glued_memmap_to_ram(data_set))
}

/// Given a yaml file on disk, it builds a point cloud. Minimal example below.
/// ```yaml
/// ---
/// data_path: DATAMEMMAP
/// labels_path: LABELS_CSV
/// count: NUMBER_OF_DATA_POINTS
/// data_dim: 784
/// label_csv_index: 2
/// ```
pub fn labels_from_yaml<P: AsRef<Path>>(path: P) -> PointCloudResult<SmallIntLabels> {
    info!("Opening labels yaml with path {:?}", &path.as_ref());
    let config = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Unable to read config file {:?}", &path.as_ref()));
    let path: &Path = path.as_ref();
    let params_files = &YamlLoader::load_from_str(&config).unwrap()[0];

    trace!(
        "Label path list, pre glob: {:?}",
        params_files["labels_path"]
    );
    let labels_path = &get_file_list(
        params_files["labels_path"]
            .as_str()
            .expect("Unable to read the 'labels_path'"),
        path,
    );
    trace!("Label path list, post glob: {:?}", labels_path);

    let labels_index = params_files["labels_index"].as_i64().map(|i| i as usize);
    let labels_dim = params_files["labels_dim"].as_i64().map(|i| i as usize);

    let mut label_set: Vec<SmallIntLabels> = labels_path
        .iter()
        .map(|path| {
            info!("Opening label file with path {:?}", path);
            match (
                path.extension().unwrap().to_str().unwrap(),
                labels_index,
                labels_dim,
            ) {
                ("csv", Some(index), _) | ("gz", Some(index), _) => open_int_csv(&path, index),
                ("dat", _, Some(dim)) => {
                    let labels: VecLabels = DataMemmap::<L2>::new(dim, &path)?.convert_to_labels();

                    match dim.cmp(&1) {
                        Ordering::Greater => Ok(labels.one_hot_to_int()),
                        Ordering::Less => {
                            panic!(
                                "Could not determine if labels are one hot or binary. {:?}, {:?}",
                                path, dim
                            );
                        }
                        Ordering::Equal => Ok(labels.binary_to_int()),
                    }
                }
                _ => panic!(
                    "Unable to detemine label source. {:?}, index: {:?}, dim: {:?}",
                    path, labels_index, labels_dim
                ),
            }
        })
        .collect::<PointCloudResult<Vec<SmallIntLabels>>>()?;

    Ok(label_set
        .drain(0..)
        .reduce(|mut a, b| {
            a.merge(&b);
            a
        })
        .unwrap())
}

fn get_file_list(files_reg: &str, yaml_path: &Path) -> Vec<PathBuf> {
    let options = MatchOptions {
        case_sensitive: false,
        ..Default::default()
    };
    let mut paths = Vec::new();
    let glob_paths;
    let files_reg_path = Path::new(files_reg);
    if files_reg_path.is_absolute() {
        trace!("label path is absolute {:?}", files_reg_path);
        glob_paths = match glob_with(&files_reg_path.to_str().unwrap(), options) {
            Ok(expr) => expr,
            Err(e) => panic!("Pattern reading error {:?}", e),
        };
    } else {
        trace!(
            "label path is not absolute, joining it with the yaml path files: {:?} , yaml: {:?}",
            files_reg_path,
            yaml_path
        );
        glob_paths = match glob_with(
            &yaml_path
                .parent()
                .unwrap()
                .join(files_reg_path)
                .to_str()
                .unwrap(),
            options,
        ) {
            Ok(expr) => expr,
            Err(e) => panic!("Pattern reading error {:?}", e),
        };
    }

    for entry in glob_paths {
        let path = match entry {
            Ok(expr) => expr,
            Err(e) => panic!("Error reading path {:?}", e),
        };
        paths.push(path)
    }
    paths
}
