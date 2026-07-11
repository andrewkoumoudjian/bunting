#!/usr/bin/env python3
"""Calibrate short-horizon queue and direction signals from the live adapter feed."""

from __future__ import annotations

import argparse
import json
import time
import urllib.parse
import urllib.request
import warnings
from pathlib import Path

import numpy as np
import pandas as pd
import statsmodels.api as sm
from statsmodels.tools.sm_exceptions import ConvergenceWarning, PerfectSeparationWarning


def fetch_json(base_url: str, path: str, params: dict[str, object] | None = None) -> object:
    query = ""
    if params:
        query = "?" + urllib.parse.urlencode(params)
    with urllib.request.urlopen(f"{base_url}{path}{query}", timeout=5) as response:
        return json.load(response)


def sample_snapshots(
    base_url: str,
    ticker: str,
    samples: int,
    sleep_seconds: float,
) -> pd.DataFrame:
    rows: list[dict[str, float | int]] = []
    last_tick: int | None = None

    while len(rows) < samples:
        case = fetch_json(base_url, "/v1/case")
        assert isinstance(case, dict)
        tick = int(case["tick"])
        if last_tick == tick:
            time.sleep(sleep_seconds)
            continue

        sec = fetch_json(base_url, "/v1/securities", {"ticker": ticker})
        book = fetch_json(base_url, "/v1/securities/book", {"ticker": ticker, "limit": 1})
        assert isinstance(sec, list) and sec
        assert isinstance(book, dict)

        sec0 = sec[0]
        bids = book.get("bids", [])
        asks = book.get("asks", [])
        if not bids or not asks:
            time.sleep(sleep_seconds)
            continue

        bid = float(bids[0]["price"])
        ask = float(asks[0]["price"])
        bid_size = float(bids[0]["quantity"])
        ask_size = float(asks[0]["quantity"])
        mid = (bid + ask) / 2.0
        spread = ask - bid
        denom = bid_size + ask_size
        qi = bid_size / denom if denom > 0 else 0.5
        micro = (ask * bid_size + bid * ask_size) / denom if denom > 0 else mid

        rows.append(
            {
                "tick": tick,
                "mid": mid,
                "spread": spread,
                "bid": bid,
                "ask": ask,
                "bid_size": bid_size,
                "ask_size": ask_size,
                "qi": qi,
                "micro_edge": micro - mid,
                "volume": float(sec0["volume"]),
                "position": float(sec0["position"]),
            }
        )
        last_tick = tick
        time.sleep(sleep_seconds)

    frame = pd.DataFrame(rows).sort_values("tick").reset_index(drop=True)
    frame["next_mid"] = frame["mid"].shift(-1)
    frame["next_up"] = (frame["next_mid"] > frame["mid"]).astype(int)
    frame["volume_delta"] = frame["volume"].diff().fillna(0.0)
    frame["qi_centered"] = frame["qi"] - 0.5
    frame["spread_ticks"] = frame["spread"] / 0.01
    frame["micro_edge_ticks"] = frame["micro_edge"] / 0.01
    return frame.iloc[:-1].copy()


def fit_statsmodels_direction(frame: pd.DataFrame) -> tuple[sm.Logit, object, dict[str, float]]:
    features = frame[["qi_centered", "micro_edge_ticks", "spread_ticks", "volume_delta"]].copy()
    features["volume_delta"] = np.log1p(features["volume_delta"].clip(lower=0.0))
    design = sm.add_constant(features, has_constant="add")
    model = sm.Logit(frame["next_up"], design)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", PerfectSeparationWarning)
        warnings.simplefilter("always", ConvergenceWarning)
        result = model.fit(disp=False)

    should_regularize = any(
        isinstance(warning.message, (PerfectSeparationWarning, ConvergenceWarning))
        for warning in caught
    )
    if should_regularize or not np.isfinite(float(result.prsquared)):
        result = model.fit_regularized(alpha=1e-3, L1_wt=0.0, disp=False)

    probs = result.predict(design)
    brier = float(np.mean((probs - frame["next_up"]) ** 2))
    accuracy = float(np.mean((probs >= 0.5) == frame["next_up"]))
    pseudo_r2 = float(getattr(result, "prsquared", float("nan")))

    metrics = {
        "samples": float(len(frame)),
        "brier_score": brier,
        "accuracy": accuracy,
        "pseudo_r2": pseudo_r2,
    }
    return model, result, metrics


def fit_pymc_direction(frame: pd.DataFrame) -> dict[str, float] | None:
    try:
        import arviz as az
        import pymc as pm
    except ImportError:
        return None

    features = frame[["qi_centered", "micro_edge_ticks", "spread_ticks", "volume_delta"]].copy()
    features["volume_delta"] = np.log1p(features["volume_delta"].clip(lower=0.0))
    x = features.to_numpy(dtype=float)
    y = frame["next_up"].to_numpy(dtype=int)

    with pm.Model() as model:
        alpha = pm.Normal("alpha", mu=0.0, sigma=1.0)
        beta = pm.Normal("beta", mu=0.0, sigma=1.0, shape=x.shape[1])
        logits = alpha + pm.math.dot(x, beta)
        pm.Bernoulli("next_up", logit_p=logits, observed=y)
        idata = pm.sample(
            draws=500,
            tune=500,
            chains=2,
            cores=1,
            target_accept=0.9,
            progressbar=False,
            random_seed=42,
        )

    summary = az.summary(idata, var_names=["alpha", "beta"])
    out = {"bayes_alpha_mean": float(summary.loc["alpha", "mean"])}
    for index, name in enumerate(features.columns):
        out[f"bayes_beta_{name}_mean"] = float(summary.loc[f"beta[{index}]", "mean"])
    return out


def maybe_log_mlflow(
    experiment_name: str,
    params: dict[str, float],
    metrics: dict[str, float],
    artifacts: dict[str, Path],
) -> str | None:
    try:
        import mlflow
    except ImportError:
        return None

    mlflow.set_experiment(experiment_name)
    with mlflow.start_run(run_name="microstructure-calibration"):
        mlflow.log_params(params)
        mlflow.log_metrics(metrics)
        for artifact in artifacts.values():
            mlflow.log_artifact(str(artifact))
        return mlflow.active_run().info.run_id


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:9999")
    parser.add_argument("--ticker", default="ALGO")
    parser.add_argument("--samples", type=int, default=120)
    parser.add_argument("--sleep-seconds", type=float, default=0.10)
    parser.add_argument("--experiment-name", default="rit-microstructure")
    parser.add_argument(
        "--output-dir",
        default="artifacts/calibration",
        help="Directory for raw samples and fitted coefficients.",
    )
    parser.add_argument(
        "--with-bayes",
        action="store_true",
        help="Run the PyMC logistic calibration when pymc is available.",
    )
    args = parser.parse_args()

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    frame = sample_snapshots(
        base_url=args.base_url,
        ticker=args.ticker,
        samples=args.samples,
        sleep_seconds=args.sleep_seconds,
    )

    _, result, metrics = fit_statsmodels_direction(frame)
    params = {
        "const": float(result.params["const"]),
        "beta_qi_centered": float(result.params["qi_centered"]),
        "beta_micro_edge_ticks": float(result.params["micro_edge_ticks"]),
        "beta_spread_ticks": float(result.params["spread_ticks"]),
        "beta_volume_delta": float(result.params["volume_delta"]),
    }

    bayes = fit_pymc_direction(frame) if args.with_bayes else None
    if bayes:
        params.update(bayes)

    samples_path = output_dir / "microstructure_samples.csv"
    params_path = output_dir / "microstructure_params.json"
    summary_path = output_dir / "statsmodels_summary.txt"

    frame.to_csv(samples_path, index=False)
    params_path.write_text(json.dumps({"params": params, "metrics": metrics}, indent=2))
    summary_path.write_text(result.summary().as_text())

    run_id = maybe_log_mlflow(
        experiment_name=args.experiment_name,
        params=params,
        metrics=metrics,
        artifacts={
            "samples": samples_path,
            "params": params_path,
            "summary": summary_path,
        },
    )

    print(json.dumps({"params": params, "metrics": metrics, "mlflow_run_id": run_id}, indent=2))


if __name__ == "__main__":
    main()
